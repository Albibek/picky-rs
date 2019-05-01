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
        dispatch.add(Method::GET, "/chain/<api-key>", chains);
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
    let repo = &controller_data.repos;

    if let Ok(body) = String::from_utf8(req.body().clone()) {
        if let Ok(json) = serde_json::from_str::<Value>(body.as_ref()) {
            let mut ca = json["ca"].to_string();
            ca = ca.trim_matches('"').to_string();
            let mut csr = json["csr"].to_string().trim_matches('"').replace("\\n", "\n").to_string();
            csr = csr.trim_matches('"').to_string();

            if let Ok(ca) = repo.find(ca.trim_matches('"')) {
                if ca.len() > 0{
                    if let Ok(ca_cert) = repo.get_cert(&ca[0].value, Some(CertFormat::Pem as u8)){
                        if let Ok(ca_key) = repo.get_key(&ca[0].value){
                            if let Some(cert) = CoreController::generate_certificate_from_csr(&String::from_utf8_lossy(&ca_cert).to_string(), &String::from_utf8_lossy(&ca_key).to_string(), controller_data.config.key_config.hash_type, &csr){
                                res.body(cert);
                                res.status(StatusCode::OK);
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
            match repos.get_cert(multihash, Some(CertFormat::from(format.to_string()) as u8)) {
                Ok(ca_cert) => {
                    if format.to_lowercase() == "der"{
                        res.body(ca_cert);
                    } else {
                        res.body(format!("{}{}{}", CERT_PREFIX, String::from_utf8_lossy(&ca_cert), CERT_SUFFIX));
                    }
                    res.status(StatusCode::OK);
                },
                Err(e) => {
                    if let Ok(multihash) = sha256_to_multihash(multihash) {
                        if let Ok(ca_cert) = repos.get_cert(&multihash, Some(CertFormat::from(format.to_string()) as u8)){
                            if format.to_lowercase() == "der"{
                                res.body(ca_cert);
                            } else {
                                res.body(format!("{}{}{}", CERT_PREFIX, String::from_utf8_lossy(&ca_cert), CERT_SUFFIX));
                            }
                            res.status(StatusCode::OK);
                        }
                    }
                }
            }
        }
    }
}

pub fn chains(controller_data: &ControllerData, req: &SyncRequest, res: &mut SyncResponse){
    res.status(StatusCode::BAD_REQUEST);
    let repos = &controller_data.repos;

    if let Some(common_name) = req.captures().get("api-key").and_then(|c| base64::decode_config(c, URL_SAFE_NO_PAD).ok()){
        let decoded = String::from_utf8_lossy(&common_name);

        if let Ok(intermediate) = repos.find(decoded.clone().trim_matches('"').trim_matches('\0')) {
            if intermediate.len() > 0{
                if let Ok(cert) = repos.get_cert(&intermediate[0].value, Some(CertFormat::Pem as u8)){
                    let mut pem = format!("{}{}{}", CERT_PREFIX, String::from_utf8_lossy(&cert), CERT_SUFFIX);
                    let mut chain = pem.clone();

                    let mut key_identifier = String::default();
                    loop {
                        if let Ok(aki) = CoreController::get_key_identifier(&pem, AUTHORITY_KEY_IDENTIFIER_OID){
                            if key_identifier == aki{
                                break;
                            }

                            key_identifier = aki.clone();

                            if let Ok(hash) = repos.get_hash_from_key_identifier(&aki){
                                if let Ok(cert) = repos.get_cert(&hash, Some(CertFormat::Pem as u8)){
                                    pem = format!("{}{}{}", CERT_PREFIX, String::from_utf8_lossy(&cert), CERT_SUFFIX);
                                    chain.push_str(&pem.clone());
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
        if let Ok(ski) = CoreController::get_key_identifier(&root.certificate_pem, SUBJECT_KEY_IDENTIFIER){
            if let Err(e) = repos.store(&root.common_name.clone(), &root.certificate_pem.clone(), &root.keys.key_pem.clone() , &ski.clone()){
                return Err(format!("Insertion error: {:?}", e));
            }
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

    if let Ok(root_cert) = repos.get_cert(&root[0].value, Some(CertFormat::Pem as u8)){
        if let Ok(root_key) = repos.get_key(&root[0].value){
            if let Some(intermediate) = CoreController::generate_intermediate_ca(&String::from_utf8_lossy(&root_cert).to_string(), &String::from_utf8_lossy(&root_key).to_string(), &config.realm, config.key_config.hash_type, config.key_config.key_type){
                if let Ok(ski) = CoreController::get_key_identifier(&intermediate.certificate_pem, SUBJECT_KEY_IDENTIFIER){
                    if let Err(e) = repos.store(&intermediate.common_name.clone(), &intermediate.certificate_pem.clone(), &intermediate.keys.key_pem.clone() , &ski.clone()){
                        return Err(format!("Insertion error: {:?}", e));
                    }
                }

                return Ok(true)
            }
        }
    }

    Err("Error while creating intermediate".to_string())
}

/*pub fn rebuild(repos: &mut Box<BackendStorage>) -> Result<(), String>{
    if let Ok(result) = repos.rebuild(){
        for res in result{
            if let Ok(ski) = CoreController::get_key_identifier(&res.1, SUBJECT_KEY_IDENTIFIER){
                if let Err(e) = repos.store(&res.0, &res.1, &res.2, &ski){
                    return Err(e);
                }
            }
        }
    }

    Ok(())
}*/