use saphir::*;
use serde_json;
use serde_json::Value;
use base64::URL_SAFE_NO_PAD;
use base64::STANDARD;

use crate::configuration::ServerConfig;
use picky_core::controllers::core_controller::CoreController;
use crate::db::backend::BackendStorage;
use crate::utils::*;
use std::str;

use crate::http::controllers::utils::{SyncRequestUtil};

pub enum CertFormat{
    Der = 0,
    Pem = 1
}

pub struct ControllerData{
    pub repos: Box<dyn BackendStorage>,
    pub config: ServerConfig
}

pub struct ServerController{
    dispatch: ControllerDispatch<ControllerData>
}

impl ServerController {
    pub fn new(repos: Box<dyn BackendStorage>, config: ServerConfig) -> Self{
        let controller_data = ControllerData{
            repos,
            config
        };

        let dispatch = ControllerDispatch::new(controller_data);
        dispatch.add(Method::GET, "/chain/<ca>", chain);
        dispatch.add(Method::GET, "/chain/", chain_default);
        dispatch.add(Method::POST, "/signcert/", sign_cert);
        dispatch.add(Method::POST, "/name/", request_name);
        dispatch.add(Method::GET, "/health/", health);
        dispatch.add(Method::GET, "/cert/<format>/<multihash>", cert_old);
        dispatch.add(Method::GET, "/cert/<multihash>", cert);
        dispatch.add(Method::POST, "/cert/", post_cert);

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

pub fn health(controller_data: &ControllerData, _req: &SyncRequest, res: &mut SyncResponse){
    if let Ok(_) = controller_data.repos.health() {
        res.status(StatusCode::OK).body("Everything should be alright!");
    } else {
        res.status(StatusCode::SERVICE_UNAVAILABLE);
    }

}

pub fn post_cert(controller_data: &ControllerData, req: &SyncRequest, res: &mut SyncResponse){
    res.status(StatusCode::BAD_REQUEST);

    let ca = format!("{} Authority", &controller_data.config.realm);
    let certificate;

    // If ContentType == application/pkcs10 get the certificate in Binary or Base64
    // If ContentType == application/json get the CA and the certificate
    let content_type = if let Some(content_type) = req.get_header_string_value("Content-Type") {
        content_type
    } else {
        error!("Content-Type is needed");
        return;
    };

    match content_type.to_lowercase().as_str() {
        "application/pkcs10" => {
            let content_encoding = if let Some(content_encoding) = req.get_header_string_value("Content-Transfer-Encoding") {
                content_encoding
            } else {
                error!("Content-Transfer-Encoding is needed with content-type: application/pkcs10");
                return;
            };

            certificate = match content_encoding.to_lowercase().as_str() {
                "base64" => {
                    if let Ok(body) = String::from_utf8(req.body().clone()){
                        fix_pem(body.as_ref())
                    } else {
                        error!("error invalid utf8 body");
                        return;
                    }
                },
                "binary" => {
                    fix_pem(der_to_pem(req.body()).as_ref())
                },
                _ => String::new(),
            };

            if certificate.eq("") {
                error!("Content-Transfer-Encoding is only supported with base64 or binary");
                return;
            }
        }
        "application/json" => {
            if let Ok(body) = String::from_utf8(req.body().clone()){
                if let Ok(json) = serde_json::from_str::<Value>(body.as_ref()) {
                    certificate = fix_pem(json["certificate"].to_string().trim_matches('"').replace("\\n", "\n").to_string().as_ref());
                } else {
                    error!("error parsing the body as json");
                    return;
                }
            } else {
                error!("error invalid utf8 body");
                return;
            }
        }
        _ => {
            error!("Content-Type not supported");
            return;
        }
    }

    let der = match pem_to_der(&certificate) {
        Ok(der) => der,
        Err(source) => {
            error!("Couldn't parse pem: {}", source);
            return;
        }
    };

    let ski = match CoreController::get_subject_key_identifier(&der) {
        Ok(ski) => {
            ski
        },
        Err(e) => {
            error!("error: {}", e);
            return;
        },
    };

    let name = match CoreController::get_issuer_name(&der){
        Ok(name) => {
            name
        },
        Err(e) => {
            error!("error: {}", e);
            return;
        },
    };

    let name = name.trim_start_matches("CN=");
    if name == ca {
        match CoreController::get_subject_name(&der) {
            Ok(common_name) => {
                if let Err(e) = controller_data.repos.store(&common_name, &der, None, &ski) {
                    error!("Insertion error for leaf {}: {}", &common_name, e);
                } else{
                    res.status(StatusCode::OK);
                }
            },
            Err(e) => {
                error!("error: {}", e);
            },
        };
    }
    else{
        error!("this certificate was not signed by the CA of this server.");
    }
}

pub fn sign_cert(controller_data: &ControllerData, req: &SyncRequest, res: &mut SyncResponse){
    res.status(StatusCode::BAD_REQUEST);

    let mut ca = format!("{} Authority", &controller_data.config.realm);
    let csr;

    // If ContentType == application/pkcs10 get the CSR in Binary or Base64
    // If ContentType == application/json get the CA and the CSR
    if let Some(content_type) = req.get_header_string_value("Content-Type") {
        if content_type.to_lowercase().eq("application/pkcs10") {
            if let Some(content_encoding) = req.get_header_string_value("Content-Transfer-Encoding") {
                csr = match content_encoding.to_lowercase().as_str() {
                    "base64" => {
                        if let Ok(body) = String::from_utf8(req.body().clone()){
                            body
                        }
                        else{
                            error!("error invalid utf8 body");
                            return;
                        }
                    },
                    "binary" => {
                        // TODO temporary
                        let pem = der_to_pem(req.body()).to_owned();
                        format!("-----BEGIN CERTIFICATE REQUEST-----\n{}\n-----END CERTIFICATE REQUEST-----", pem)
                    },
                    _ => String::new(),
                };

                if csr.eq("") {
                    error!("Content-Transfer-Encoding is only supported with base64 or binary");
                    return;
                }
            } else {
                error!("Content-Transfer-Encoding is needed with content-type: application/pkcs10");
                return;
            }
        } else if content_type.to_lowercase().eq("application/json") {
            if let Ok(body) = String::from_utf8(req.body().clone()){
                if let Ok(json) = serde_json::from_str::<Value>(body.as_ref()) {
                    if !(json["ca"].is_null()) {
                        ca = json["ca"].to_string();
                        ca = ca.trim_matches('"').to_string();
                    }

                    csr = json["csr"].to_string().trim_matches('"').replace("\\n", "\n").to_string();
                } else {
                    error!("error parsing the body as json");
                    return;
                }
            }
            else {
                error!("error invalid utf8 body");
                return;
            }
        }
        else {
            error!("Content-Type not supported");
            return;
        }
    } else {
        error!("Content-Type is needed");
        return;
    }

    //Sign CSR
    if let Ok(ca) = controller_data.repos.find(ca.trim_matches('"')) {
        if ca.len() > 0 {
            if let Ok(ca_cert) = controller_data.repos.get_cert(&ca[0].value) {
                if let Ok(ca_key) = controller_data.repos.get_key(&ca[0].value) {
                    if let Some(cert) = CoreController::generate_certificate_from_csr(&ca_cert, &ca_key, controller_data.config.key_config.hash_type, &csr) {
                        // Save certificate in backend if needed
                        if controller_data.config.save_certificate {
                            if let Ok(ski) = CoreController::get_subject_key_identifier(&cert.certificate_der) {
                                if let Err(e) = controller_data.repos.store(&cert.common_name.clone(), &cert.certificate_der, None, &ski.clone()) {
                                    error!("Insertion error for leaf {}: {}", cert.common_name.clone(), e);
                                }
                            }
                        }

                        res.body(fix_pem(&der_to_pem(&cert.certificate_der)));
                        res.status(StatusCode::OK);
                    } else {
                        error!("generate certificate from csr error");
                    }
                } else {
                    error!("get key with ca error");
                }
            } else {
                error!("get cert with ca error");
            }
        } else {
            error!("ca length error");
        }
    } else {
        error!("{} CA can't be found in backend", ca)
    }
}

pub fn cert_old(controller_data: &ControllerData, req: &SyncRequest, res: &mut SyncResponse){
    res.status(StatusCode::BAD_REQUEST);
    let repos = &controller_data.repos;

    if let Some(multihash) = req.captures().get("multihash"){
        if let Some(format) = req.captures().get("format"){
            match repos.get_cert(multihash) {
                Ok(ca_cert) => {
                    if (CertFormat::from(format.to_string()) as u8) == 0{
                        res.body(ca_cert);
                    } else {
                        res.body(fix_pem(&der_to_pem(&ca_cert)));
                    }
                    res.status(StatusCode::OK);
                },
                Err(e) => {
                    if let Ok(multihash) = sha256_to_multihash(multihash) {
                        if let Ok(ca_cert) = repos.get_cert(&multihash){
                            if (CertFormat::from(format.to_string()) as u8) == 0{
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

pub fn set_content_type_body(req: &SyncRequest, res: &mut SyncResponse, ca_cert:Vec<u8>){
    if let Some(content_type) = req.get_header_string_value("Accept-Encoding") {
        if content_type.to_lowercase().eq("binary") {
            res.body(ca_cert);
        } else if content_type.to_lowercase().eq("base64") {
            res.body(base64::encode_config(&ca_cert, STANDARD));
        } else {
            res.body(fix_pem(&der_to_pem(&ca_cert)));
        }
    }
    else{
        res.body(fix_pem(&der_to_pem(&ca_cert)));
    }
    res.status(StatusCode::OK);
}

pub fn cert(controller_data: &ControllerData, req: &SyncRequest, res: &mut SyncResponse){
    res.status(StatusCode::BAD_REQUEST);

    if let Some(multihash) = req.captures().get("multihash"){
        match controller_data.repos.get_cert(multihash) {
            Ok(ca_cert) => {
                set_content_type_body(req, res , ca_cert);
            },
            Err(e) => {
                if let Ok(multihash) = sha256_to_multihash(multihash) {
                    if let Ok(ca_cert) = controller_data.repos.get_cert(&multihash){
                        set_content_type_body(req, res , ca_cert);
                    }
                    else{
                        error!("{}", e);
                    }
                } else {
                    error!("{}", e);
                }
            }
        }
    }
}

pub fn find_ca_chain(controller_data: &ControllerData, ca:String, res: &mut SyncResponse){
    let repos = &controller_data.repos;

    match repos.find(ca.clone().trim_matches('"').trim_matches('\0')) {
        Ok(intermediate) => {
            if intermediate.len() > 0 {
                match repos.get_cert(&intermediate[0].value) {
                    Ok(cert) => {
                        let mut chain = fix_pem(&der_to_pem(&cert.clone()));

                        let mut key_identifier = String::default();
                        loop {
                            match CoreController::get_authority_key_identifier(&cert) {
                                Ok(aki) => {
                                    if key_identifier == aki {
                                        // The authority is itself. It is a root
                                        break;
                                    }

                                    key_identifier = aki.clone();

                                    match repos.get_hash_from_key_identifier(&aki) {
                                        Ok(hash) => {
                                            match repos.get_cert(&hash) {
                                                Ok(cert) => {
                                                    chain.push_str(&fix_pem(&der_to_pem(&cert.clone())));
                                                }
                                                Err(e) => {
                                                    error!("repos.get_cert failed: {}", e);
                                                    break;
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            error!("repos.get_hash_from_key_identifier {} failed: {}", aki, e);
                                            break;
                                        }
                                    }
                                }
                                Err(e) => {
                                    error!("get_key_identifier failed: {}", e);
                                    break;
                                }
                            }
                        }
                        res.body(chain.to_string());
                        res.status(StatusCode::OK);
                    }
                    Err(e) => {
                        error!("Intermediate cert can't be found: {}", e);
                    }
                }
            } else {
                error!("No intermediate found!");
            }
        }
        Err(e) => {
            error!("No intermediate found: {}", e);
        }
    }
}

pub fn chain_default(controller_data: &ControllerData, _: &SyncRequest, res: &mut SyncResponse){
    res.status(StatusCode::BAD_REQUEST);
    let ca = format!("{} Authority", &controller_data.config.realm);
    find_ca_chain(controller_data, ca, res);
}

pub fn chain(controller_data: &ControllerData, req: &SyncRequest, res: &mut SyncResponse){
    res.status(StatusCode::BAD_REQUEST);

    if let Some(common_name) = req.captures().get("ca").and_then(|c| base64::decode_config(c, URL_SAFE_NO_PAD).ok()){
        let decoded = String::from_utf8_lossy(&common_name);

        find_ca_chain(controller_data, decoded.into(), res);

    } else {
        error!("Wrong path or can't decode base64: {}", req.captures().get("ca").unwrap_or(&"No capture ca".to_string()));
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

pub fn generate_root_ca(config: &ServerConfig, repos: &mut Box<dyn BackendStorage>) -> Result<bool, String>{
    if let Ok(certs) = repos.find(&format!("{} Root CA", config.realm)) {
        if certs.len() > 0 {
            return Ok(false);
        }
    }

    if let Some(root) = CoreController::generate_root_ca(&config.realm, config.key_config.hash_type, config.key_config.key_type){
        let ski = CoreController::get_subject_key_identifier(&root.certificate_der)?;
        if let Err(e) = repos.store(&root.common_name.clone(), &root.certificate_der, Some(&root.keys.expect("Could not store root key, key is empty").key_der), &ski.clone()){
            return Err(format!("Insertion error: {:?}", e));
        }
    }

    Ok(true)
}

pub fn generate_intermediate(config: &ServerConfig, repos: &mut Box<dyn BackendStorage>) -> Result<bool, String>{
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
            if let Some(intermediate) = CoreController::generate_intermediate_ca(&root_cert, &root_key, &config.realm, config.key_config.hash_type, config.key_config.key_type){
                if let Ok(ski) = CoreController::get_subject_key_identifier(&intermediate.certificate_der) {
                    if let Err(e) = repos.store(&intermediate.common_name.clone(), &intermediate.certificate_der, Some(&intermediate.keys.expect("Could not store intermediate key, key is empty").key_der), &ski.clone()){
                        return Err(format!("Insertion error: {:?}", e));
                    }
                    return Ok(true)
                }
            }
        }
    }

    Err("Error while creating intermediate".to_string())
}

pub fn check_certs_in_env(config: &ServerConfig, repos: &mut Box<dyn BackendStorage>) -> Result<(), String> {
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

fn get_and_store_env_cert_info(cert: &str, key: &str, repos: &mut Box<dyn BackendStorage>) -> Result<(), String>{
    let der = pem_to_der(&cert)?;
    match CoreController::get_subject_key_identifier(&der) {
        Ok(ski) => {
            match CoreController::get_subject_name(&der){
                Ok(name) => {
                    let name = name.trim_start_matches("CN=");
                    if let Err(e) = repos.store(name, &pem_to_der(&cert).expect("Error converting certificate to DER format"), Some(&pem_to_der(&key).expect("Error converting key to DER format")), &ski){
                        return Err(e);
                    }
                    return Ok(());
                },
                Err(e) => return Err(e)
            }
        },
        Err(e) => return Err(e)
    };
}