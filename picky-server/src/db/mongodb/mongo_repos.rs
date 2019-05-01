use bson::Bson;
use bson::Document;
use bson::{to_bson, from_bson};
use serde::{Deserialize, Serialize};

use crate::utils;
use crate::db::backend::{BackendStorage, Repo, Model};
use crate::db::mongodb::mongo_connection::MongoConnection;
use crate::db::mongodb::mongo_repo::MongoRepo;
use crate::utils::{pem_to_der, multihash_to_string, u8_to_i8_vec, i8_to_u8_vec, der_to_pem};

const REPO_CERTIFICATE: &str = "Certificate Store";
const REPO_KEY: &str = "Key Store";
const REPO_CERTNAME: &str = "Name Store";
const REPO_CERTKEY: &str = "Key Identifier Store";
const OLD_REPO_NAME: &str = "certificate";

#[allow(dead_code)]
#[derive(Debug)]
pub enum RepositoryError {
    BsonEncodeError(bson::EncoderError),
    BsonDecodeError(bson::DecoderError),
    MongoError(mongodb::Error),
    UninitializedRepoError,
    InsertError,
    UpdateError,
    Other(String),
}

impl From<String> for RepositoryError {
    fn from(e: String) -> Self {
        RepositoryError::Other(e)
    }
}

impl From<bson::EncoderError> for RepositoryError {
    fn from(e: bson::EncoderError) -> Self {
        RepositoryError::BsonEncodeError(e)
    }
}

impl From<bson::DecoderError> for RepositoryError {
    fn from(e: bson::DecoderError) -> Self {
        RepositoryError::BsonDecodeError(e)
    }
}

impl From<mongodb::Error> for RepositoryError {
    fn from(e: mongodb::Error) -> Self {
        RepositoryError::MongoError(e)
    }
}

#[derive(Clone)]
pub struct MongoRepos{
    db_instance: MongoConnection,
    pub name: MongoRepo<String>,
    pub certificates: MongoRepo<Vec<i8>>,
    pub keys: MongoRepo<Vec<i8>>,
    pub key_identifiers: MongoRepo<String>
}

impl MongoRepos{
    pub fn new(db: MongoConnection) -> Self{
        MongoRepos{
            db_instance: db,
            name: Default::default(),
            certificates: Default::default(),
            keys: Default::default(),
            key_identifiers: Default::default()
        }
    }

    pub fn load_repositories(&mut self) -> Result<(), RepositoryError> {
        self.name.init(self.db_instance.clone(), REPO_CERTNAME)?;
        self.certificates.init(self.db_instance.clone(), REPO_CERTIFICATE)?;
        self.keys.init(self.db_instance.clone(), REPO_KEY)?;
        self.key_identifiers.init(self.db_instance.clone(), REPO_CERTKEY)?;
        Ok(())
    }
}

impl BackendStorage for MongoRepos{
    fn init(&mut self) -> Result<(), String>{
        if let Err(e) = self.load_repositories(){
            return Err(format!("{:?}", e));
        }

        Ok(())
    }

    fn store(&mut self, name: &str, cert: &[u8], key: &[u8], key_identifier: &str) -> Result<bool, String>{
        if let Ok(mut cert_hash) = utils::multihash_encode(cert){
                self.name.insert(name, multihash_to_string(&cert_hash))?;
                self.certificates.insert(&multihash_to_string(&cert_hash), u8_to_i8_vec(cert))?;
                self.keys.insert(&multihash_to_string(&cert_hash), u8_to_i8_vec(key))?;
                self.key_identifiers.insert(key_identifier, multihash_to_string(&cert_hash))?;
                return Ok(true);
        }

        Err("Can\'t encode certificate".to_string())
    }

    fn find(&self, name: &str) -> Result<Vec<Model<String>>, String>{
        let doc = doc!{"key": name};
        let mut model_vec = Vec::new();
        let document_cursor = match self.name.get_collection()?.find(Some(doc), None){
            Ok(d) => d,
            Err(e) => return Err(e.to_string())
        };

        for doc_res in document_cursor{
            if let Ok(model_document) = doc_res {
                if let Ok(model) = from_bson(Bson::Document(model_document)) {
                    model_vec.push(model);
                }
            }
        }
        Ok(model_vec)
    }

    fn get_cert(&self, hash: &str, format: Option<u8>) -> Result<Vec<u8>, String>{
        let doc = doc!{"key": hash};
        let mut model_vec: Vec<Model<Vec<i8>>> = Vec::new();
        let document_cursor = match self.certificates.get_collection()?.find(Some(doc), None){
            Ok(d) => d,
            Err(e) => return Err(e.to_string())
        };

        for doc_res in document_cursor{
            if let Ok(model_document) = doc_res {
                if let Ok(model) = from_bson(Bson::Document(model_document)) {
                    model_vec.push(model);
                }
            }
        }

        if model_vec.len() > 0 {
            if let Some(f) = format{
                if f == 1 {
                    return Ok(i8_to_u8_vec(&model_vec[0].value));
                } else {
                    return Ok(i8_to_u8_vec(&model_vec[0].value));
                }
            }
            return Ok(i8_to_u8_vec(&model_vec[0].value));
        }

        Err("Error finding cert".to_string())
    }

    fn get_key(&self, hash: &str) -> Result<Vec<u8>, String>{
        let doc = doc!{"key": hash};
        let mut model_vec: Vec<Model<Vec<i8>>> = Vec::new();
        let document_cursor = match self.keys.get_collection()?.find(Some(doc), None){
            Ok(d) => d,
            Err(e) => return Err(e.to_string())
        };

        for doc_res in document_cursor{
            if let Ok(model_document) = doc_res {
                if let Ok(model) = from_bson(Bson::Document(model_document)) {
                    model_vec.push(model);
                }
            }
        }

        if model_vec.len() > 0 {
            return Ok(i8_to_u8_vec(&model_vec[0].value));
        }

        Err("Error finding key".to_string())
    }

    fn get_key_identifier_from_hash(&self, hash: &str) -> Result<String, String>{
        let hash = hash.to_string();
        let storage: Model<String>;
        let doc = doc!{"value": hash};

        let opt = match self.key_identifiers.get_collection()?.find_one(Some(doc), None){
            Ok(hash) => hash,
            Err(e) => return Err(e.to_string())
        };

        if let Some(model_doc) = opt{
            if let Ok(model) = from_bson(Bson::Document(model_doc)){
                storage = model;
                return Ok(storage.key);
            }
        }

        Err("No key identifier found".to_string())
    }

    fn get_hash_from_key_identifier(&self, key_identifier: &str) -> Result<String, String>{
        let key_identifier= key_identifier.to_string();
        let storage: Model<String>;
        let doc = doc!{"key": key_identifier};

        let opt = match self.key_identifiers.get_collection()?.find_one(Some(doc), None){
            Ok(hash) => hash,
            Err(e) => return Err(e.to_string())
        };

        if let Some(model_doc) = opt{
            if let Ok(model) = from_bson(Bson::Document(model_doc)){
                storage = model;
                return Ok(storage.value);
            }
        }

        Err("No hash found".to_string())
    }

    fn clone_box(&self) -> Box<BackendStorage>{
        Box::new(self.clone())
    }

    /*fn rebuild(&mut self) -> Result<Vec<(String, String, String)>, ()>{
        let mut old_repo: MongoRepo = Default::default();
        old_repo.init(self.db_instance.clone(), OLD_REPO_NAME);
        if let Ok(coll) = old_repo.get_collection(){
            if let Ok(cursor) = coll.find(None, None){
                let mut datas = Vec::new();

                for res in cursor{
                    let mut common_name = String::default();
                    let mut cert_pem = String::default();
                    let mut key = String::default();

                    if let Ok(item) = res{
                        if let Some(&Bson::String(ref cert)) = item.get("cert") {
                            cert_pem = cert.to_string();

                            if let Some(&Bson::String(ref cn)) = item.get("common_name") {
                                common_name = cn.to_string();

                                if let Some(&Bson::String(ref k)) = item.get("key") {
                                    key = k.to_string();

                                    datas.push((common_name, cert_pem, key));
                                }
                            }
                        }
                    }
                }

                if datas.len() > 0 {
                    return Ok(datas);
                }
            }
        }

        info!("Could not find old picky repo. Initializing new certificates...");
        Err(())
    }*/
}