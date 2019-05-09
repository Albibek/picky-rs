use saphir::*;
use serde_json;
use serde_json::Value;
use base64::URL_SAFE_NO_PAD;

use crate::configuration::ServerConfig;
use picky_core::controllers::core_controller::CoreController;
use crate::db::backend::BackendStorage;
use crate::utils::*;

const CERT_PREFIX: &str = "-----BEGIN CERTIFICATE-----\n";
const CERT_SUFFIX: &str = "\n-----END CERTIFICATE-----\n";
const KEY_PREFIX: &str = "-----BEGIN RSA PRIVATE KEY-----\n";
const KEY_SUFFIX: &str = "\n-----END RSA PRIVATE KEY-----";
const SUBJECT_KEY_IDENTIFIER: &[u64] = &[2, 5, 29, 14];
const AUTHORITY_KEY_IDENTIFIER_OID: &[u64] = &[2, 5, 29, 35];

pub enum CertFormat{
    Der = 0,
    Pem = 1
}

pub struct ControllerData{
    pub repos: Box<BackendStorage>,
    pub config: ServerConfig
}

pub struct ServerController{
    dispatch: ControllerDispatch<ControllerData>
}

impl ServerController {
    pub fn new(repos: Box<BackendStorage>, config: ServerConfig) -> Self{
        let controller_data = ControllerData{
            repos,
            config
        };

        let dispatch = ControllerDispatch::new(controller_data);
        dispatch.add(Method::GET, "/chain/<ca>", chains);
        dispatch.add(Method::POST, "/signcert/", sign_cert);
        dispatch.add(Method::POST, "/name/", request_name);
        dispatch.add(Method::GET, "/health/", health);
        dispatch.add(Method::GET, "/cert/<format>/<multihash>", cert);

        ServerController {
            dispatch
        }
    }
}

impl Controller for ServerController{
    fn handle(&self, req: &mut SyncRequest, res: &mut SyncResponse){
        self.dispatch.dispatch(req, res);
    }

    fn base_path(&self) -> &str{
        "/"
    }
}

impl From<String> for CertFormat{
    fn from(format: String) -> Self{
        if format.to_lowercase().eq("der"){
            return CertFormat::Der;
        } else {
            return CertFormat::Pem;
        }
    }
}

pub fn health(_controller_data: &ControllerData, _req: &SyncRequest, res: &mut SyncResponse){
    res.status(StatusCode::OK).body("Everything should be alright!");
}

pub fn sign_cert(controller_data: &ControllerData, req: &SyncRequest, res: &mut SyncResponse){
    res.status(StatusCode::BAD_REQUEST);
    let repos = &mut controller_data.repos.clone();

    if let Ok(body) = String::from_utf8(req.body().clone()) {
        if let Ok(json) = serde_json::from_str::<Value>(body.as_ref()) {
            let mut ca = json["ca"].to_string();
            ca = ca.trim_matches('"').to_string();
            let mut csr = json["csr"].to_string().trim_matches('"').replace("\\n", "\n").to_string();
            csr = csr.trim_matches('"').to_string();

            if let Ok(ca) = repos.find(ca.trim_matches('"')) {
                if ca.len() > 0{
                    if let Ok(ca_cert) = repos.get_cert(&ca[0].value){
                        if let Ok(ca_key) = repos.get_key(&ca[0].value){
                            if let Some(cert) = CoreController::generate_certificate_from_csr(&pem_to_der(&ca_cert).unwrap(), &pem_to_der(&ca_key).unwrap(), controller_data.config.key_config.hash_type, &csr){
                                if let Ok(ski) = CoreController::get_key_identifier(&cert.certificate_der, SUBJECT_KEY_IDENTIFIER){
                                    let pem = format!("{}{}{}", CERT_PREFIX, &der_to_pem(&cert.certificate_der), CERT_SUFFIX);
                                    if let Err(e) = repos.store(&cert.common_name.clone(), &pem , None, &ski.clone()){
                                        return error!("{}",&format!("Insertion error for leaf {}: {}", &cert.common_name.clone(), e));
                                    }
                                    res.body(fix_pem(&pem));
                                    res.status(StatusCode::OK);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

pub fn cert(controller_data: &ControllerData, req: &SyncRequest, res: &mut SyncResponse){
    res.status(StatusCode::BAD_REQUEST);
    let repos = &controller_data.repos;

    if let Some(multihash) = req.captures().get("multihash"){
        if let Some(format) = req.captures().get("format"){
            match repos.get_cert(multihash) {
                Ok(ca_cert) => {
                    if (CertFormat::from(format.to_string()) as u8) == 0{
                        res.body(pem_to_der(&ca_cert).unwrap());
                    } else {
                        res.body(ca_cert);
                    }
                    res.status(StatusCode::OK);
                },
                Err(e) => {
                    if let Ok(multihash) = sha256_to_multihash(multihash) {
                        if let Ok(ca_cert) = repos.get_cert(&multihash){
                            if (CertFormat::from(format.to_string()) as u8) == 0{
                                res.body(pem_to_der(&ca_cert).unwrap());
                            } else {
                                res.body(fix_pem(&ca_cert));
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

pub fn chains(controller_data: &ControllerData, req: &SyncRequest, res: &mut SyncResponse){
    res.status(StatusCode::BAD_REQUEST);
    let repos = &controller_data.repos;

    if let Some(common_name) = req.captures().get("ca").and_then(|c| base64::decode_config(c, URL_SAFE_NO_PAD).ok()){
        let decoded = String::from_utf8_lossy(&common_name);

        if let Ok(intermediate) = repos.find(decoded.clone().trim_matches('"').trim_matches('\0')) {
            if intermediate.len() > 0{
                if let Ok(cert) = repos.get_cert(&intermediate[0].value){
                    let mut chain = fix_pem(&cert.clone());

                    let mut key_identifier = String::default();
                    loop {
                        if let Ok(aki) = CoreController::get_key_identifier(&pem_to_der(&cert).unwrap(), AUTHORITY_KEY_IDENTIFIER_OID){
                            if key_identifier == aki{
                                break;
                            }

                            key_identifier = aki.clone();

                            if let Ok(hash) = repos.get_hash_from_key_identifier(&aki){
                                if let Ok(cert) = repos.get_cert(&hash){
                                    chain.push_str(&fix_pem(&cert.clone()));
                                } else {
                                    break;
                                }
                            } else {
                                break;
                            }
                        } else {
                            break;
                        }
                    }
                    res.body(chain.to_string());
                    res.status(StatusCode::OK);
                }
            }
        }
    }
}

pub fn request_name(_controller_data: &ControllerData, req: &SyncRequest, res: &mut SyncResponse){
    res.status(StatusCode::BAD_REQUEST);

    if let Ok(body) = String::from_utf8(req.body().clone()) {
        if let Ok(json) = serde_json::from_str::<Value>(body.as_ref()){
            let csr = json["csr"].to_string().trim_matches('"').replace("\\n", "\n");
            if let Ok(common_name) = CoreController::request_name(&csr){
                res.body(common_name);
                res.status(StatusCode::OK);
            }
        }
    }
}

pub fn generate_root_ca(config: &ServerConfig, repos: &mut Box<BackendStorage>) -> Result<bool, String>{
    if let Ok(certs) = repos.find(&format!("{} Root CA", config.realm)) {
        if certs.len() > 0 {
            return Ok(false);
        }
    }

    if let Some(root) = CoreController::generate_root_ca(&config.realm, config.key_config.hash_type, config.key_config.key_type){
        let ski = CoreController::get_key_identifier(&root.certificate_der, SUBJECT_KEY_IDENTIFIER)?;
        if let Err(e) = repos.store(&root.common_name.clone(), &format!("{}{}{}", CERT_PREFIX, &der_to_pem(&root.certificate_der.clone()), CERT_SUFFIX), Some(&format!("{}{}{}", KEY_PREFIX, &der_to_pem(&root.keys.key_der), KEY_SUFFIX)), &ski.clone()){
            return Err(format!("Insertion error: {:?}", e));
        }
    }

    Ok(true)
}

pub fn generate_intermediate(config: &ServerConfig, repos: &mut Box<BackendStorage>) -> Result<bool, String>{
    if let Ok(certs) = repos.find(&format!("{} Authority", config.realm)) {
        if certs.len() > 0 {
            return Ok(false);
        }
    }

    let root = match repos.find(&format!("{} Root CA", config.realm)){
        Ok(r) => r,
        Err(e) => {
            return Err(format!("Could not find root: {}", e));
        }
    };

    if let Ok(root_cert) = repos.get_cert(&root[0].value){
        if let Ok(root_key) = repos.get_key(&root[0].value){
            if let Some(intermediate) = CoreController::generate_intermediate_ca(&pem_to_der(&root_cert).unwrap(), &pem_to_der(&root_key).unwrap(), &config.realm, config.key_config.hash_type, config.key_config.key_type){
                if let Ok(ski) = CoreController::get_key_identifier(&intermediate.certificate_der, SUBJECT_KEY_IDENTIFIER){
                    if let Err(e) = repos.store(&intermediate.common_name.clone(), &format!("{}{}{}", CERT_PREFIX, &der_to_pem(&intermediate.certificate_der), CERT_SUFFIX), Some(&format!("{}{}{}", KEY_PREFIX, &der_to_pem(&intermediate.keys.key_der), KEY_SUFFIX)), &ski.clone()){
                        return Err(format!("Insertion error: {:?}", e));
                    }
                    return Ok(true)
                }
            }
        }
    }

    Err("Error while creating intermediate".to_string())
}

pub fn check_certs_in_env(config: &ServerConfig, repos: &mut Box<BackendStorage>) -> Result<(), String> {
    if !config.root_cert.is_empty() && !config.root_key.is_empty() {
        if let Err(e) = get_and_store_env_cert_info(&config.root_cert, &config.root_key, repos) {
            return Err(e);
        }
    }

    if !config.intermediate_cert.is_empty() && !config.intermediate_key.is_empty() {
        if let Err(e) = get_and_store_env_cert_info(&config.intermediate_cert, &config.intermediate_key, repos) {
            return Err(e);
        }
    }

    Ok(())
}

fn get_and_store_env_cert_info(cert: &str, key: &str, repos: &mut Box<BackendStorage>) -> Result<(), String>{
    let der = pem_to_der(&cert)?;
    match CoreController::get_key_identifier(&der, SUBJECT_KEY_IDENTIFIER) {
        Ok(ski) => {
            match CoreController::get_subject_name(&der){
                Ok(name) => {
                    let cert = format!("{}{}{}", CERT_PREFIX, der_to_pem(&der), CERT_SUFFIX);
                    let name = name.trim_start_matches("CN=");
                    let key = key.replace(KEY_PREFIX, "").replace(KEY_SUFFIX, "");
                    if let Err(e) = repos.store(name, &cert, Some(&key), &ski){
                        return Err(e);
                    }
                    return Ok(());
                },
                Err(e) => return Err(e)
            }
        },
        Err(e) => return Err(e)
    };

    Err("Error while fetching certificate info".to_string())
}

#[cfg(test)]
mod tests{
    use super::*;
    use crate::utils;

    static PEM: &'static str = "-----BEGIN CERTIFICATE-----
MIIFHDCCAwSgAwIBAgIAMA0GCSqGSIb3DQEBCwUAMCAxHjAcBgNVBAMMFUNOPW1
5X2Rlbi5sb2wgUm9vdCBDQTAeFw0xOTA0MjYxOTU3NDFaFw0yNDA0MjQxOTU3ND
FaMB8xHTAbBgNVBAMMFG15X2Rlbi5sb2wgQXV0aG9yaXR5MIICIjANBgkqhkiG9
w0BAQEFAAOCAg8AMIICCgKCAgEA1dnnBcD5rQ70DG/hn/iPxBZ/ppwDHeDK4bzZ
fHASOka+CzP7hc3NW0ppUt8Atj++2hOu1GR6TsJegRILkrJ9dxfOMdjoxpAWcmc
qM9vtmZOkC2RlaV5b/GtB52aQTyJF227axD0rhF+Vga55+B20XStyUwoLdJ3Tnf
iil6FWeLQNisM7sCntRe/EbzVpvc2IU+TPjsNomZYJA/Yl6Wl2Qzp4g7eRKg2DP
ZrRwiYpphuv5r0BCI8K/X1CZP18FJF6+QFDXeo0L3g8E8HIa0r3N7Yr48jd7oYr
HJHXoXmFbnQYr1x+tsj1vd91cJHXHhDAEFZuzi27PbDg+Otp38Quuiu7MPTmGac
NQAMIQzxasAf3Qm3mafIU0TRmJ7dXHlsKxjzM2OiYlLXwdIFqk/nXO/1ZSNd45s
w8Mv0ruG3Br1LPLpdw3DW49DO1T6GPFWHtY1bm5uULG3U7lJe5vzsSJ9uL3jBpT
RaYvM3+wSC0L1HPmvl1GPSmDjeafu2tSRFqptnZiQc8vuRt+pIOxjuTkxxn40WB
E+iLGjkXD1VWA6XdhT6M+Tt2Zfgl83gtOmh1o2z4jm4P1QJ4v0NHc81wOZ2ksqF
cWVDA3J3t1Um2yUfw0VxirI+ytWiAC8lzwfwnVzT8H9WIuAgcpidujxdYhnbf0W
FCsZOR/Fv81k6opVMCAwEAAaNjMGEwDwYDVR0TBAgwBgEB/wIBADAOBgNVHQ8BA
f8EBAMCAa4wHQYDVR0OBBYEFJo+UnDnuGNchrYBKXO3gNvgNCf2MB8GA1UdIwQY
MBaAFPgx7if1NT16dUqpl9iVdLyRNC9pMA0GCSqGSIb3DQEBCwUAA4ICAQA7tlP
sZhoSiIjJGfpsO+XBWZbnHLIQ8a+Cn0V1oWyOspP4jLOTT7efUQYZWIzuk3IMkb
eK71U2PDIpTSvUHAUchtNKl8YcBSU6TAPKdrk3TGb1UvglMVi+xkaVYpUYYnN+L
peeyKrN4TE/qbTiju0RYH9vo6Y68G0kZVVU5ievoqpi3tOaa0BIdTBKEvwSrmm/
lQTruPAB9rGCI95sAvsmtYJIsPfaQZA3vAxoWlOrwfh3VkMoXB1QSPFt9okXpxZ
SGE1zpnBjvreuDjSS3HmIxQBYwy4TNQ3duUnDOJAFQvnhLoUzTDprXpmDnXqqLq
ZYtpU06DYuHVIOuPGIpipUl5182YS1iCSXl2RyfbYTk2+qRYlbUkUmHVgnJMA8a
uOWhKWtXdi5eJiiSciVAYpBwFXJeSCMYuBQRHaUsXcu55i+jlfDiBVZOZkYgpje
iOoyJEjTw9KFlPIHMC2qMmPkOlQjGK+CHXMY3kwFZcpz2CgRBSgVvN7Mb+Val38
Kpskn+WYe7umSp9k0laSvJghxUGYXpVxGwNCiyojsAMUoSJ7xUx5bjfOFOL7SWC
+juKXytSs4iWqXN9igFBLPd54pj6wdAI5FieHsP6PwaM8Bt20BlJsCa1nj1uR9o
dK9RO0Wys/X1CAeFnsen7+BVKFvjx0CHZuiNgdTE+BbYBTfgg==
-----END CERTIFICATE-----";

    #[test]
    fn key_id_and_cert_test(){
        let kid = "9a3e5270e7b8635c86b6012973b780dbe03427f6";
        let cert = utils::pem_to_der(PEM).unwrap();

        let key_id = CoreController::get_key_identifier(&cert, &[2, 5, 29, 14]).unwrap();
        assert_eq!(&key_id, kid);
    }
}