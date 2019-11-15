use crate::{error::Result, models::key::PublicKey};
use err_ctx::ResultExt;
use serde_asn1_der::asn1_wrapper::BitStringAsn1Container;
use sha1::{Digest, Sha1};
use sha2::{Sha224, Sha256, Sha384, Sha512};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyIdHashAlgo {
    Sha1,
    Sha224,
    Sha256,
    Sha384,
    Sha512,
}

/// Describes which method to use to generate key identifiers
///
/// https://tools.ietf.org/html/rfc5280#section-4.2.1.2
/// https://tools.ietf.org/html/rfc7093#section-2
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum KeyIdGenMethod {
    /// Hash the leftmost 160-bits of the
    /// SHA-256 hash of the value of the BIT STRING subjectPublicKey
    /// (excluding the tag, length, and number of unused bits)
    SPKValueHashedLeftmost160(KeyIdHashAlgo),
    /// Hash the DER encoding of the SubjectPublicKeyInfo value
    SPKFullDER(KeyIdHashAlgo),
}

macro_rules! hash {
    ( @ $algorithm:ident, $input:ident) => {{
        let mut digest = $algorithm::new();
        digest.input($input);
        digest.result().as_slice().to_vec()
    }};
    ($hash_algo:ident, $input:ident) => {
        match $hash_algo {
            KeyIdHashAlgo::Sha1 => hash!( @ Sha1, $input ),
            KeyIdHashAlgo::Sha224 => hash!( @ Sha224, $input ),
            KeyIdHashAlgo::Sha256 => hash!( @ Sha256, $input ),
            KeyIdHashAlgo::Sha384 => hash!( @ Sha384, $input ),
            KeyIdHashAlgo::Sha512 => hash!( @ Sha512, $input ),
        }
    };
}

impl KeyIdGenMethod {
    pub fn generate_from(&self, public_key: &PublicKey) -> Result<Vec<u8>> {
        use crate::serde::subject_public_key_info::PublicKey as InnerPublicKey;
        match self {
            KeyIdGenMethod::SPKValueHashedLeftmost160(hash_algo) => {
                match &public_key.as_inner().subject_public_key {
                    InnerPublicKey::RSA(BitStringAsn1Container(rsa_pk)) => {
                        let der = serde_asn1_der::to_vec(rsa_pk)?;
                        Ok(hash!(hash_algo, der)[..160].to_vec())
                    }
                    InnerPublicKey::EC(bitstring) => {
                        let der = bitstring.0.payload_view();
                        Ok(hash!(hash_algo, der)[..160].to_vec())
                    }
                }
            }
            KeyIdGenMethod::SPKFullDER(hash_algo) => {
                let der = public_key
                    .to_der()
                    .ctx("couldn't serialize subject public key info to der")?;
                Ok(hash!(hash_algo, der))
            }
        }
    }
}
