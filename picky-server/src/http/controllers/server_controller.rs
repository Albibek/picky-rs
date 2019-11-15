use crate::{
    configuration::ServerConfig, db::backend::BackendStorage,
    http::controllers::utils::SyncRequestUtil, utils::*,
};
use base64::{STANDARD, URL_SAFE_NO_PAD};
use picky::{
    controller::Picky,
    models::{certificate::Cert, csr::Csr, key::PrivateKey},
    pem::{parse_pem, to_pem, Pem},
};
use saphir::*;
use serde_json::{self, Value};

enum CertFormat {
    Der = 0,
    Pem = 1,
}

struct ControllerData {
    pub repos: Box<dyn BackendStorage>,
    pub config: ServerConfig,
}

pub struct ServerController {
    dispatch: ControllerDispatch<ControllerData>,
}

impl ServerController {
    pub fn new(repos: Box<dyn BackendStorage>, config: ServerConfig) -> Self {
        let controller_data = ControllerData { repos, config };

        let dispatch = ControllerDispatch::new(controller_data);
        dispatch.add(Method::GET, "/chain/<ca>", chain);
        dispatch.add(Method::GET, "/chain/", chain_default);
        dispatch.add(Method::POST, "/signcert/", sign_cert);
        dispatch.add(Method::POST, "/name/", request_name);
        dispatch.add(Method::GET, "/health/", health);
        dispatch.add(Method::GET, "/cert/<format>/<multihash>", cert_old);
        dispatch.add(Method::GET, "/cert/<multihash>", cert);
        dispatch.add(Method::POST, "/cert/", post_cert);

        ServerController { dispatch }
    }
}

impl Controller for ServerController {
    fn handle(&self, req: &mut SyncRequest, res: &mut SyncResponse) {
        self.dispatch.dispatch(req, res);
    }

    fn base_path(&self) -> &str {
        "/"
    }
}

impl From<String> for CertFormat {
    fn from(format: String) -> Self {
        if format.to_lowercase().eq("der") {
            CertFormat::Der
        } else {
            CertFormat::Pem
        }
    }
}

macro_rules! saphir_try {
    ( $result:expr ) => {
        saphir_try!($result, "Error")
    };
    ( $result:expr , $context:literal $(,)? ) => {
        match $result {
            Ok(value) => value,
            Err(e) => {
                error!(concat!($context, ": {}"), e);
                return;
            }
        }
    };
}

macro_rules! unwrap_opt {
    ( $opt:expr , $error:literal $(,)? ) => {
        match $opt {
            Some(value) => value,
            None => {
                error!($error);
                return;
            }
        }
    };
}

fn health(controller_data: &ControllerData, _req: &SyncRequest, res: &mut SyncResponse) {
    if controller_data.repos.health().is_ok() {
        res.status(StatusCode::OK)
            .body("Everything should be alright!");
    } else {
        res.status(StatusCode::SERVICE_UNAVAILABLE);
    }
}

fn post_cert(controller_data: &ControllerData, req: &SyncRequest, res: &mut SyncResponse) {
    res.status(StatusCode::BAD_REQUEST);

    let content_type = unwrap_opt!(
        req.get_header_string_value("Content-Type"),
        "Content-Type is required",
    );

    let cert = match content_type.to_lowercase().as_str() {
        "application/pkcs10" => {
            let content_encoding = unwrap_opt!(
                req.get_header_string_value("Content-Transfer-Encoding"),
                "Content-Transfer-Encoding is required for content-type: application/pkcs10"
            );

            match content_encoding.to_lowercase().as_str() {
                "base64" => {
                    let pem = saphir_try!(parse_pem(req.body()), "(base64) couldn't parse pem");
                    saphir_try!(
                        Cert::from_der(pem.data()),
                        "(base64) couldn't deserialize certificate"
                    )
                }
                "binary" => saphir_try!(
                    Cert::from_der(req.body()),
                    "(binary) couldn't deserialize certificate"
                ),
                unsupported => {
                    error!("Unsupported Content-Transfer-Encoding: {}", unsupported);
                    return;
                }
            }
        }
        "application/json" => {
            let json = saphir_try!(
                serde_json::from_slice::<Value>(req.body()),
                "(json) couldn't parse json"
            );
            let pem = saphir_try!(
                json["certificate"]
                    .to_string()
                    .trim_matches('"')
                    .replace("\\n", "\n")
                    .parse::<Pem>(),
                "(json) couldn't parse pem",
            );
            saphir_try!(
                Cert::from_der(pem.data()),
                "(json) couldn't deserialize certificate"
            )
        }
        unsupported => {
            error!("Unsupported Content-Type: {}", unsupported);
            return;
        }
    };

    let ski = hex::encode(saphir_try!(
        cert.subject_key_identifier(),
        "couldn't fetch SKI"
    ));

    let issuer_name = cert.issuer_name().to_string();
    if issuer_name != format!("CN={} Authority", &controller_data.config.realm) {
        error!("this certificate was not signed by the CA of this server.");
        return;
    }

    let der = saphir_try!(cert.to_der(), "couldn't serialize certificate into der");
    let subject_name = cert.subject_name().to_string();
    if let Err(e) = controller_data.repos.store(&subject_name, &der, None, &ski) {
        error!("Insertion error for leaf {}: {}", subject_name, e);
    } else {
        res.status(StatusCode::OK);
    }
}

fn sign_cert(controller_data: &ControllerData, req: &SyncRequest, res: &mut SyncResponse) {
    res.status(StatusCode::BAD_REQUEST);

    let content_type = unwrap_opt!(
        req.get_header_string_value("Content-Type"),
        "Content-Type is required",
    );

    let mut ca_name = format!("{} Authority", &controller_data.config.realm);

    let csr = match content_type.to_lowercase().as_str() {
        "application/pkcs10" => {
            let content_encoding = unwrap_opt!(
                req.get_header_string_value("Content-Transfer-Encoding"),
                "Content-Transfer-Encoding is required with content-type: application/pkcs10"
            );

            match content_encoding.to_lowercase().as_str() {
                "base64" => {
                    let pem = saphir_try!(parse_pem(req.body()), "(base64) couldn't parse pem");
                    saphir_try!(
                        Csr::from_der(pem.data()),
                        "(base64) couldn't deserialize certificate"
                    )
                }
                "binary" => saphir_try!(
                    Csr::from_der(&req.body()),
                    "(binary) couldn't deserialize certificate"
                ),
                unsupported => {
                    error!("Unsupported Content-Transfer-Encoding: {}", unsupported);
                    return;
                }
            }
        }
        "application/json" => {
            let json = saphir_try!(
                serde_json::from_slice::<Value>(req.body()),
                "(json) couldn't parse json"
            );

            if let Some(ca) = json["ca"].as_str() {
                ca_name = ca.trim_matches('"').to_owned();
            }

            let pem = saphir_try!(
                json["csr"]
                    .to_string()
                    .trim_matches('"')
                    .replace("\\n", "\n")
                    .parse::<Pem>(),
                "(json) couldn't parse pem",
            );
            saphir_try!(
                Csr::from_der(pem.data()),
                "(json) couldn't deserialize certificate"
            )
        }
        unsupported => {
            error!("Unsupported Content-Type: {}", unsupported);
            return;
        }
    };

    // Sign CSR
    let signed_cert = saphir_try!(sign_certificate(
        &ca_name,
        csr,
        &controller_data.config,
        controller_data.repos.as_ref()
    ));

    let pem = saphir_try!(signed_cert.to_pem(), "couldn't get certificate pem");
    res.body(pem.to_string());
    res.status(StatusCode::OK);
}

fn cert_old(controller_data: &ControllerData, req: &SyncRequest, res: &mut SyncResponse) {
    res.status(StatusCode::BAD_REQUEST);
    let repos = &controller_data.repos;

    if let Some(multihash) = req.captures().get("multihash") {
        if let Some(format) = req.captures().get("format") {
            match repos.get_cert(multihash) {
                Ok(ca_cert) => {
                    if (CertFormat::from(format.to_string()) as u8) == 0 {
                        res.body(ca_cert);
                    } else {
                        res.body(fix_pem(&der_to_pem(&ca_cert)));
                    }
                    res.status(StatusCode::OK);
                }
                Err(e) => {
                    if let Ok(multihash) = sha256_to_multihash(multihash) {
                        if let Ok(ca_cert) = repos.get_cert(&multihash) {
                            if (CertFormat::from(format.to_string()) as u8) == 0 {
                                res.body(ca_cert);
                            } else {
                                res.body(fix_pem(&der_to_pem(&ca_cert)));
                            }
                            res.status(StatusCode::OK);
                        }
                    } else {
                        error!("{}", e);
                    }
                }
            }
        }
    }
}

fn set_content_type_body(req: &SyncRequest, res: &mut SyncResponse, ca_cert: Vec<u8>) {
    if let Some(content_type) = req.get_header_string_value("Accept-Encoding") {
        if content_type.to_lowercase().eq("binary") {
            res.body(ca_cert);
        } else if content_type.to_lowercase().eq("base64") {
            res.body(base64::encode_config(&ca_cert, STANDARD));
        } else {
            res.body(fix_pem(&der_to_pem(&ca_cert)));
        }
    } else {
        res.body(fix_pem(&der_to_pem(&ca_cert)));
    }
    res.status(StatusCode::OK);
}

fn cert(controller_data: &ControllerData, req: &SyncRequest, res: &mut SyncResponse) {
    res.status(StatusCode::BAD_REQUEST);

    if let Some(multihash) = req.captures().get("multihash") {
        match controller_data.repos.get_cert(multihash) {
            Ok(ca_cert) => {
                set_content_type_body(req, res, ca_cert);
            }
            Err(e) => {
                if let Ok(multihash) = sha256_to_multihash(multihash) {
                    if let Ok(ca_cert) = controller_data.repos.get_cert(&multihash) {
                        set_content_type_body(req, res, ca_cert);
                    } else {
                        error!("{}", e);
                    }
                } else {
                    error!("{}", e);
                }
            }
        }
    }
}

fn sign_certificate(
    ca_name: &str,
    csr: Csr,
    config: &ServerConfig,
    repos: &dyn BackendStorage,
) -> Result<Cert, String> {
    let ca_hashes = repos
        .find(ca_name)
        .map_err(|e| format!("couldn't fetch CA: {}", e))?;
    let ca_hash = if ca_hashes.is_empty() {
        return Err("ca hash empty".to_owned());
    } else {
        &ca_hashes[0].value
    };

    let ca_cert_der = repos
        .get_cert(ca_hash)
        .map_err(|e| format!("couldn't get CA cert der: {}", e))?;
    let ca_cert =
        Cert::from_der(&ca_cert_der).map_err(|e| format!("couldn't deserialize CA cert: {}", e))?;

    let ca_pk_der = repos
        .get_key(ca_hash)
        .map_err(|e| format!("couldn't fetch CA private key: {}", e))?;
    let ca_pk = PrivateKey::from_pkcs8(&ca_pk_der)
        .map_err(|e| format!("couldn't build private key from pkcs8: {}", e))?;

    let signed_cert = Picky::generate_leaf_from_csr(
        csr,
        ca_cert.subject_name().clone(),
        &ca_pk,
        config.key_config,
    )
    .map_err(|e| format!("couldn't generate leaf certificate: {}", e))?;

    if config.save_certificate {
        let name = signed_cert.subject_name().to_string();
        let cert_der = signed_cert
            .to_der()
            .map_err(|e| format!("couldn't serialize certificate to der: {}", e))?;
        let ski = hex::encode(
            signed_cert
                .subject_key_identifier()
                .map_err(|e| format!("couldn't get SKI: {}", e))?,
        );

        repos
            .store(&name, &cert_der, None, &ski)
            .map_err(|e| format!("Insertion error for leaf {}: {}", name, e))?;
    }

    Ok(signed_cert)
}

fn find_ca_chain(repos: &dyn BackendStorage, ca_name: &str) -> Result<Vec<String>, String> {
    let ca_hash = repos
        .find(ca_name)
        .map_err(|e| format!("couldn't fetch CA hash id for {}: {}", ca_name, e))?;
    let ca_hash = if ca_hash.is_empty() {
        return Err("No intermediate certificate found!".to_owned());
    } else {
        &ca_hash[0].value
    };

    let mut cert_der = repos
        .get_cert(ca_hash)
        .map_err(|e| format!("couldn't fetch CA certificate der: {}", e))?;
    let mut chain = vec![to_pem("CERTIFICATE", &cert_der)];
    let mut current_key_id = String::default();
    loop {
        let cert = Cert::from_der(&cert_der)
            .map_err(|e| format!("couldn't deserialize certificate: {}", e))?;

        let parent_key_id = hex::encode(
            cert.authority_key_identifier()
                .map_err(|e| format!("couldn't fetch authority key identifier: {}", e))?,
        );

        if current_key_id == parent_key_id {
            // The authority is itself. It is a root.
            break;
        }

        let hash = repos
            .get_hash_from_key_identifier(&parent_key_id)
            .map_err(|e| format!("couldn't fetch hash: {}", e))?;

        cert_der = repos
            .get_cert(&hash)
            .map_err(|e| format!("couldn't fetch certificate der: {}", e))?;

        chain.push(to_pem("CERTIFICATE", &cert_der));

        current_key_id = parent_key_id;
    }

    Ok(chain)
}

fn chain_default(controller_data: &ControllerData, _: &SyncRequest, res: &mut SyncResponse) {
    res.status(StatusCode::BAD_REQUEST);
    let ca = format!("{} Authority", &controller_data.config.realm);
    let chain = saphir_try!(find_ca_chain(controller_data.repos.as_ref(), &ca));
    res.body(chain.join("\n"));
    res.status(StatusCode::OK);
}

fn chain(controller_data: &ControllerData, req: &SyncRequest, res: &mut SyncResponse) {
    res.status(StatusCode::BAD_REQUEST);

    if let Some(common_name) = req
        .captures()
        .get("ca")
        .and_then(|c| base64::decode_config(c, URL_SAFE_NO_PAD).ok())
    {
        let decoded = String::from_utf8_lossy(&common_name);
        let chain = saphir_try!(find_ca_chain(controller_data.repos.as_ref(), &decoded));
        res.body(chain.join("\n"));
        res.status(StatusCode::OK);
    } else {
        error!(
            "Wrong path or can't decode base64: {}",
            req.captures()
                .get("ca")
                .unwrap_or(&"No capture ca".to_string())
        );
    }
}

fn request_name(_: &ControllerData, req: &SyncRequest, res: &mut SyncResponse) {
    res.status(StatusCode::BAD_REQUEST);

    let body = saphir_try!(
        std::str::from_utf8(req.body()),
        "couldn't parse body as utf8"
    );
    let json = saphir_try!(serde_json::from_str::<Value>(body), "couldn't parse json");
    let csr_pem = saphir_try!(
        json["csr"]
            .to_string()
            .trim_matches('"')
            .replace("\\n", "\n")
            .parse::<Pem>(),
        "couldn't parse pem"
    );
    let csr = saphir_try!(Csr::from_der(csr_pem.data()), "couldn't deserialize CSR");
    let subject_name = csr.subject_name().to_string().replace("CN=", "");

    res.body(subject_name);
    res.status(StatusCode::OK);
}

pub fn generate_root_ca(config: &ServerConfig, repos: &dyn BackendStorage) -> Result<bool, String> {
    let name = format!("{} Root CA", config.realm);

    if let Ok(certs) = repos.find(&name) {
        if !certs.is_empty() {
            // already exists
            return Ok(false);
        }
    }

    let pk =
        generate_private_key(4096).map_err(|e| format!("couldn't generate private key: {}", e))?;
    let root = Picky::generate_root(&name, config.key_config, &pk)
        .map_err(|e| format!("couldn't generate root certificate: {}", e))?;
    let ski = root
        .subject_key_identifier()
        .map_err(|e| format!("couldn't fetch subject key identifier: {}", e))?;

    let cert_der = root
        .to_der()
        .map_err(|e| format!("couldn't serialize root certificate into der: {}", e))?;

    let pk_pkcs8 = pk
        .to_pkcs8()
        .map_err(|e| format!("couldn't get private key pkcs8: {}", e))?;

    repos
        .store(&name, &cert_der, Some(&pk_pkcs8), &hex::encode(ski))
        .map_err(|e| format!("couldn't store generated root certificate: {}", e))?;

    Ok(true)
}

pub fn generate_intermediate(
    config: &ServerConfig,
    repos: &dyn BackendStorage,
) -> Result<bool, String> {
    let root_name = format!("{} Root CA", config.realm);
    let intermediate_name = format!("{} Authority", config.realm);

    if let Ok(certs) = repos.find(&intermediate_name) {
        if !certs.is_empty() {
            // already exists
            return Ok(false);
        }
    }

    let (root_cert, root_key) = match repos.find(&root_name) {
        Ok(roots) => {
            if roots.is_empty() {
                return Err("no matching root CA".to_owned());
            } else {
                (
                    repos
                        .get_cert(&roots[0].value)
                        .map_err(|e| format!("couldn't fetch root CA: {}", e))?,
                    repos
                        .get_key(&roots[0].value)
                        .map_err(|e| format!("couldn't fetch root CA private key: {}", e))?,
                )
            }
        }
        Err(e) => {
            return Err(format!("error while fetching root: {}", e));
        }
    };

    let pk = generate_private_key(2048)?;
    let root_cert = Cert::from_der(&root_cert)
        .map_err(|e| format!("couldn't parse root cert from der: {}", e))?;
    let root_key = PrivateKey::from_pkcs8(&root_key)
        .map_err(|e| format!("couldn't parse private key from pkcs8: {}", e))?;

    let intermediate_cert = Picky::generate_intermediate(
        root_cert.subject_name().clone(),
        &root_key,
        &intermediate_name,
        config.key_config,
        &pk,
    )
    .map_err(|e| format!("couldn't generate intermediate certificate: {}", e))?;

    let ski = intermediate_cert
        .subject_key_identifier()
        .map_err(|e| format!("couldn't fetch key id: {}", e))?;

    let cert_der = intermediate_cert.to_der().map_err(|e| {
        format!(
            "couldn't serialize intermediate certificate into der: {}",
            e
        )
    })?;

    let pk_pkcs8 = pk
        .to_pkcs8()
        .map_err(|e| format!("couldn't get private key pkcs8: {}", e))?;

    repos
        .store(
            &intermediate_name,
            &cert_der,
            Some(&pk_pkcs8),
            &hex::encode(ski),
        )
        .map_err(|e| format!("couldn't store generated intermediate certificate: {}", e))?;

    Ok(true)
}

pub fn check_certs_in_env(config: &ServerConfig, repos: &dyn BackendStorage) -> Result<(), String> {
    if !config.root_cert.is_empty() && !config.root_key.is_empty() {
        if let Err(e) = get_and_store_env_cert_info(&config.root_cert, &config.root_key, repos) {
            return Err(e);
        }
    }

    if !config.intermediate_cert.is_empty() && !config.intermediate_key.is_empty() {
        if let Err(e) =
            get_and_store_env_cert_info(&config.intermediate_cert, &config.intermediate_key, repos)
        {
            return Err(e);
        }
    }

    Ok(())
}

fn get_and_store_env_cert_info(
    cert_pem: &str,
    key_pem: &str,
    repos: &dyn BackendStorage,
) -> Result<(), String> {
    let cert_pem = cert_pem
        .parse::<Pem>()
        .map_err(|e| format!("couldn't parse cert pem: {}", e))?;
    let cert = Cert::from_der(cert_pem.data())
        .map_err(|e| format!("couldn't parse cert from der: {}", e))?;
    let ski = hex::encode(
        cert.subject_key_identifier()
            .map_err(|e| format!("couldn't parse fetch subject key identifier: {}", e))?,
    );
    let subject_name = cert.subject_name().to_string();

    let key_pem = key_pem
        .parse::<Pem>()
        .map_err(|e| format!("couldn't parse key pem: {}", e))?;

    repos.store(&subject_name, &cert_pem.data(), Some(key_pem.data()), &ski)?;

    Ok(())
}

// This function is also used by tests in release mode.
#[cfg(not(any(feature = "pre-gen-pk", all(debug_assertions, test))))]
fn generate_private_key(bits: usize) -> Result<PrivateKey, String> {
    PrivateKey::generate_rsa(bits).map_err(|e| format!("couldn't generate private key: {}", e))
}

// !!! DEBUGGING PURPOSE ONLY !!!
// Private Key generation is insanely slow on debug builds.
// Therefore this function (only to be used in debug profile please) doesn't generate new private keys.
// It returns a random pre-generated private key from a pool: security-wise, this is extremely bad.
#[cfg(any(feature = "pre-gen-pk", all(debug_assertions, test)))]
fn generate_private_key(bits: usize) -> Result<PrivateKey, String> {
    use crate::test_files::*;
    use rand::prelude::*;

    warn!(
        "FETCHING A PRE-GENERATED PRIVATE KEY. \
         THIS BUILD IS FOR DEBUG PURPOSE ONLY, DON'T USE THIS BUILD IN PRODUCTION."
    );

    const RSA_2048_POOL: [&str; 6] = [
        RSA_2048_PK_1,
        RSA_2048_PK_2,
        RSA_2048_PK_3,
        RSA_2048_PK_4,
        RSA_2048_PK_5,
        RSA_2048_PK_6,
    ];
    const RSA_4096_POOL: [&str; 2] = [RSA_4096_PK_1, RSA_4096_PK_2]; //, RSA_4096_PK_3]; The third key isn't supported by current RSA implementation.

    let choice: usize = random();
    let pk_pem_str = match bits {
        2048 => {
            info!(
                "Selected pk number {} from RSA_2048_POOL",
                choice % RSA_2048_POOL.len()
            );
            RSA_2048_POOL[choice % RSA_2048_POOL.len()]
        }
        4096 => {
            info!(
                "Selected pk number {} from RSA_4096_POOL",
                choice % RSA_4096_POOL.len()
            );
            RSA_4096_POOL[choice % RSA_4096_POOL.len()]
        }
        num_bits => {
            return Err(format!(
                "no pre-generated private key for {} bits key",
                num_bits
            ))
        }
    };

    let pem = pk_pem_str
        .parse::<Pem>()
        .map_err(|e| format!("couldn't parse pk pem: {}", e))?;

    PrivateKey::from_pkcs8(pem.data())
        .map_err(|e| format!("couldn't parse private key from pkcs8: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{configuration::BackendType, db::backend::Backend};
    use picky::models::{name::Name, signature::SignatureHashType};

    fn config() -> ServerConfig {
        let mut config = ServerConfig::default();
        config.backend = BackendType::Memory;
        config
    }

    #[test]
    fn generate_chain_and_verify() {
        let config = config();
        let backend = Backend::from(&config);

        let ca_name = format!("{} Authority", config.realm);

        generate_root_ca(&config, backend.db.as_ref()).expect("couldn't generate root ca");
        generate_intermediate(&config, backend.db.as_ref())
            .expect("couldn't generate intermediate ca");

        let pk = generate_private_key(2048).expect("couldn't generate private key");
        let csr = Csr::generate(
            Name::new_common_name("Mister Bushidô"),
            &pk,
            SignatureHashType::RsaSha384,
        )
        .expect("couldn't generate csr");

        let signed_cert = sign_certificate(&ca_name, csr, &config, backend.db.as_ref())
            .expect("couldn't sign certificate");

        let issuer_name = signed_cert.issuer_name().to_string().replace("CN=", "");
        let chain_pem =
            find_ca_chain(backend.db.as_ref(), &issuer_name).expect("couldn't fetch CA chain");

        assert_eq!(chain_pem.len(), 2);

        let chain = chain_pem
            .iter()
            .map(|cert_pem| {
                let pem = cert_pem.parse::<Pem>().expect("couldn't parse cert pem");
                Cert::from_der(pem.data()).expect("couldn't parse cert from der")
            })
            .collect::<Vec<Cert>>();

        assert_eq!(chain[0].subject_name().to_string(), "CN=Picky Authority");
        assert_eq!(chain[1].subject_name().to_string(), "CN=Picky Root CA");

        Picky::verify_chain(&signed_cert, chain.iter()).expect("couldn't validate ca chain");
    }
}
