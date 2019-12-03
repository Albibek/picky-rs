use crate::{
    error::{Error, Result},
    models::key::{PrivateKey, PublicKey},
    oids, serde,
    serde::AlgorithmIdentifier,
};
use picky_asn1::wrapper::{BitStringAsn1Container, OctetStringAsn1Container};
use rand::rngs::OsRng;
use rsa::{
    hash::Hashes, BigUint, PaddingScheme, PublicKey as RsaPublicKeyInterface, RSAPrivateKey,
    RSAPublicKey,
};
use sha1::{Digest, Sha1};
use sha2::{Sha224, Sha256, Sha384, Sha512};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SignatureHashType {
    RsaSha1,
    RsaSha224,
    RsaSha256,
    RsaSha384,
    RsaSha512,
}

macro_rules! hash {
    ($algorithm:ident, $input:ident) => {{
        let mut digest = $algorithm::new();
        digest.input($input);
        digest.result().as_slice().to_vec()
    }};
}

impl SignatureHashType {
    pub fn from_algorithm_identifier(
        algorithm_identifier: &serde::AlgorithmIdentifier,
    ) -> Option<Self> {
        match Into::<String>::into(&algorithm_identifier.algorithm.0).as_str() {
            oids::SHA1_WITH_RSA_ENCRYPTION => Some(Self::RsaSha1),
            oids::SHA224_WITH_RSA_ENCRYPTION => Some(Self::RsaSha224),
            oids::SHA256_WITH_RSA_ENCRYPTION => Some(Self::RsaSha256),
            oids::SHA384_WITH_RSA_ENCRYPTION => Some(Self::RsaSha384),
            oids::SHA512_WITH_RSA_ENCRYPTION => Some(Self::RsaSha512),
            _ => None,
        }
    }

    pub fn hash(self, msg: &[u8]) -> Vec<u8> {
        match self {
            Self::RsaSha1 => hash!(Sha1, msg),
            Self::RsaSha224 => hash!(Sha224, msg),
            Self::RsaSha256 => hash!(Sha256, msg),
            Self::RsaSha384 => hash!(Sha384, msg),
            Self::RsaSha512 => hash!(Sha512, msg),
        }
    }

    pub fn sign(self, msg: &[u8], private_key: &PrivateKey) -> Result<Vec<u8>> {
        let rsa_private_key = match &private_key.as_inner().private_key {
            serde::private_key_info::PrivateKeyValue::RSA(OctetStringAsn1Container(key)) => {
                RSAPrivateKey::from_components2(
                    BigUint::from_bytes_be(key.modulus().as_bytes_be()),
                    BigUint::from_bytes_be(key.public_exponent().as_bytes_be()),
                    BigUint::from_bytes_be(key.private_exponent().as_bytes_be()),
                    key.primes()
                        .iter()
                        .map(|p| BigUint::from_bytes_be(p.as_bytes_be()))
                        .collect(),
                )?
            }
        };

        let mut rng = OsRng::new().map_err(|_| Error::NoSecureRandomness)?;

        let digest = self.hash(msg);

        let hash_algo = match self {
            Self::RsaSha1 => &Hashes::SHA1,
            Self::RsaSha224 => &Hashes::SHA2_224,
            Self::RsaSha256 => &Hashes::SHA2_256,
            Self::RsaSha384 => &Hashes::SHA2_384,
            Self::RsaSha512 => &Hashes::SHA2_512,
        };

        let signature = rsa_private_key.sign_blinded(
            &mut rng,
            PaddingScheme::PKCS1v15,
            Some(hash_algo),
            &digest,
        )?;

        Ok(signature)
    }

    pub fn verify(self, public_key: &PublicKey, msg: &[u8], signature: &[u8]) -> Result<()> {
        use crate::serde::subject_public_key_info::PublicKey as InnerPublicKey;
        let public_key = match &public_key.as_inner().subject_public_key {
            InnerPublicKey::RSA(BitStringAsn1Container(key)) => RSAPublicKey::new(
                BigUint::from_bytes_be(key.modulus.as_bytes_be()),
                BigUint::from_bytes_be(key.public_exponent.as_bytes_be()),
            )?,
            InnerPublicKey::EC(_) => {
                return Err(Error::UnsupportedAlgorithm {
                    algorithm: "elliptic curves".into(),
                });
            }
        };

        let hash_algorithm = match self {
            Self::RsaSha1 => &Hashes::SHA1,
            Self::RsaSha224 => &Hashes::SHA2_224,
            Self::RsaSha256 => &Hashes::SHA2_256,
            Self::RsaSha384 => &Hashes::SHA2_384,
            Self::RsaSha512 => &Hashes::SHA2_512,
        };

        let digest = self.hash(msg);

        public_key
            .verify(
                PaddingScheme::PKCS1v15,
                Some(hash_algorithm),
                &digest,
                signature,
            )
            .map_err(|_| Error::BadSignature)?;

        Ok(())
    }
}

impl From<SignatureHashType> for AlgorithmIdentifier {
    fn from(ty: SignatureHashType) -> Self {
        match ty {
            SignatureHashType::RsaSha1 => AlgorithmIdentifier::new_sha1_with_rsa_encryption(),
            SignatureHashType::RsaSha224 => AlgorithmIdentifier::new_sha224_with_rsa_encryption(),
            SignatureHashType::RsaSha256 => AlgorithmIdentifier::new_sha256_with_rsa_encryption(),
            SignatureHashType::RsaSha384 => AlgorithmIdentifier::new_sha384_with_rsa_encryption(),
            SignatureHashType::RsaSha512 => AlgorithmIdentifier::new_sha512_with_rsa_encryption(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pem::Pem;

    #[test]
    fn unsupported_key_no_panic() {
        // Once the key is supported by the RSA crate, this test should be deleted.
        let unsupported_key = {
            let pem = crate::test_files::RSA_4096_PK_3_UNSUPPORTED
                .parse::<Pem>()
                .unwrap();
            PrivateKey::from_pkcs8(pem.data()).unwrap()
        };
        let msg = [0, 1, 2, 3, 4, 5];
        let signature_hash_type = SignatureHashType::RsaSha512;
        let err = signature_hash_type
            .sign(&msg, &unsupported_key)
            .unwrap_err();
        assert_eq!(err.to_string(), "RSA error: invalid coefficient");
    }
}
