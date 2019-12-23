use crate::{
    key::PublicKey,
    private::SubjectPublicKeyInfo,
    signature::SignatureHashType,
};
use base64::DecodeError;
use serde::{Deserialize, Serialize};
use snafu::Snafu;

// === error type === //

#[derive(Debug, Snafu)]
pub enum JwkError {
    /// Json error
    #[snafu(display("JSON error: {}", source))]
    Json { source: serde_json::Error },

    /// couldn't decode base64
    #[snafu(display("couldn't decode base64: {}", source))]
    Base64Decoding { source: DecodeError },

    /// unsupported algorithm
    #[snafu(display("unsupported algorithm: {}", algorithm))]
    UnsupportedAlgorithm { algorithm: &'static str },
}

impl From<serde_json::Error> for JwkError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json { source: e }
    }
}

impl From<DecodeError> for JwkError {
    fn from(e: DecodeError) -> Self {
        Self::Base64Decoding { source: e }
    }
}

// === key type === //

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kty")]
pub enum JwkKeyType {
    #[serde(rename = "RSA")]
    Rsa(JwkPublicRsaKey),
}

impl JwkKeyType {
    pub fn new_rsa_key(modulus: &[u8], public_exponent: &[u8]) -> Self {
        Self::Rsa(JwkPublicRsaKey {
            n: base64::encode_config(modulus, base64::URL_SAFE_NO_PAD),
            e: base64::encode_config(public_exponent, base64::URL_SAFE_NO_PAD),
        })
    }

    pub fn new_rsa_key_from_base64_url(modulus: String, public_exponent: String) -> Self {
        Self::Rsa(JwkPublicRsaKey {
            n: modulus,
            e: public_exponent,
        })
    }

    pub fn as_rsa(&self) -> Option<&JwkPublicRsaKey> {
        match self {
            JwkKeyType::Rsa(rsa) => Some(rsa),
        }
    }

    pub fn is_rsa(&self) -> bool {
        self.as_rsa().is_some()
    }
}

// === public key use === //

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum JwkPubKeyUse {
    #[serde(rename = "sig")]
    Signature,
    #[serde(rename = "enc")]
    Encryption,
}

// === key operations === //

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum JwkKeyOps {
    #[serde(rename = "sign")]
    Sign,
    #[serde(rename = "verify")]
    Verify,
    #[serde(rename = "encrypt")]
    Encrypt,
    #[serde(rename = "decrypt")]
    Decrypt,
    #[serde(rename = "wrapKey")]
    WrapKey,
    #[serde(rename = "unwrapKey")]
    UnwrapKey,
    #[serde(rename = "deriveKey")]
    DeriveKey,
    #[serde(rename = "deriveBits")]
    DeriveBits,
}

// === json web key === //

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Jwk {
    #[serde(flatten)]
    pub key: JwkKeyType,

    #[serde(rename = "alg", skip_serializing_if = "Option::is_none")]
    pub algorithm: Option<SignatureHashType>,

    #[serde(rename = "use", skip_serializing_if = "Option::is_none")]
    pub pub_key_use: Option<JwkPubKeyUse>,

    #[serde(rename = "key_ops", skip_serializing_if = "Option::is_none")]
    pub key_operations: Option<Vec<JwkKeyOps>>,

    #[serde(rename = "kid", skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,

    #[serde(rename = "x5u", skip_serializing_if = "Option::is_none")]
    pub x509_url: Option<String>,

    #[serde(rename = "x5c", skip_serializing_if = "Option::is_none")]
    pub x509_chain: Option<Vec<String>>,

    #[serde(rename = "x5t", skip_serializing_if = "Option::is_none")]
    pub x509_sha1_thumbprint: Option<String>,

    #[serde(rename = "x5t#S256", skip_serializing_if = "Option::is_none")]
    pub x509_sha256_thumbprint: Option<String>,
}

impl Jwk {
    pub fn new(key: JwkKeyType) -> Self {
        Jwk {
            key,
            algorithm: None,
            pub_key_use: None,
            key_operations: None,
            key_id: None,
            x509_url: None,
            x509_chain: None,
            x509_sha1_thumbprint: None,
            x509_sha256_thumbprint: None,
        }
    }

    pub fn from_json(json: &str) -> Result<Self, JwkError> {
        Ok(serde_json::from_str(json)?)
    }

    pub fn from_public_key(public_key: &PublicKey) -> Result<Self, JwkError> {
        use crate::private::subject_public_key_info::PublicKey as SerdePublicKey;
        use picky_asn1::wrapper::BitStringAsn1Container;

        match &public_key.as_inner().subject_public_key {
            SerdePublicKey::RSA(BitStringAsn1Container(rsa)) => Ok(Self::new(JwkKeyType::new_rsa_key(
                rsa.modulus.as_signed_bytes_be(),
                rsa.public_exponent.as_signed_bytes_be(),
            ))),
            SerdePublicKey::EC(_) => Err(JwkError::UnsupportedAlgorithm {
                algorithm: "elliptic curves",
            }),
        }
    }

    pub fn to_json(&self) -> Result<String, JwkError> {
        Ok(serde_json::to_string(self)?)
    }

    pub fn to_json_pretty(&self) -> Result<String, JwkError> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    pub fn to_public_key(&self) -> Result<PublicKey, JwkError> {
        match &self.key {
            JwkKeyType::Rsa(rsa) => {
                let spki = SubjectPublicKeyInfo::new_rsa_key(rsa.modulus()?.into(), rsa.public_exponent()?.into());
                Ok(spki.into())
            }
        }
    }
}

// === jwk set === //

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct JwkSet {
    pub keys: Vec<Jwk>,
}

impl JwkSet {
    pub fn from_json(json: &str) -> Result<Self, JwkError> {
        Ok(serde_json::from_str(json)?)
    }

    pub fn to_json(&self) -> Result<String, JwkError> {
        Ok(serde_json::to_string(self)?)
    }

    pub fn to_json_pretty(&self) -> Result<String, JwkError> {
        Ok(serde_json::to_string_pretty(self)?)
    }
}

// === public rsa key === //

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct JwkPublicRsaKey {
    n: String,
    e: String,
}

impl JwkPublicRsaKey {
    pub fn modulus(&self) -> Result<Vec<u8>, JwkError> {
        base64::decode_config(&self.n, base64::URL_SAFE_NO_PAD).map_err(JwkError::from)
    }

    pub fn public_exponent(&self) -> Result<Vec<u8>, JwkError> {
        base64::decode_config(&self.e, base64::URL_SAFE_NO_PAD).map_err(JwkError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RSA_MODULUS: &str = "rpJjxW0nNZiq1mPC3ZAxqf9qNjmKurP7XuKrpWrfv3IOUldqChQVPNg8zCvDOMZIO-ZDuRmVH\
                               EZ5E1vz5auHNACnpl6AvDGJ-4qyX42vfUDMNZx8i86d7bQpwJkO_MVMLj8qMGmTVbQ8zqVw2z\
                               MyKUFfa2V83nvx2wz4FJh2Thw2uZX2P7h8nlDVSuXO0wJ_OY_2qtqRIAnNXMzL5BF5pEFh4hi\
                               JIFiMTNkhVtUjT1QSB9E8DtDme8g4u769Oc0My45fgqSNE7kKKyaDhTfqSovyhj-qWiD-X_Gw\
                               pWkW4ungpHzz_97-ZDB3yQ7AMwKAsw5EW2cMqseAp3f-kf159w";

    const RSA_PUBLIC_EXPONENT: &str = "AQAB";

    const X509_SHA1_THUMBPRINT: &str = "N3ORVnr9T6opxpS9iRbkKGwKiQI";

    const X509_CERT_0: &str = "MIIDWjCCAkKgAwIBAgIUWRsBqKmpXGP/OwrwLWicwxhuCFowDQYJKoZIhvc\
                               NAQELBQAwKjEoMCYGA1UEAwwfbG9naW4uZGV2b2x1dGlvbnMuY29tIEF1dG\
                               hvcml0eTAeFw0xOTAzMTMxMzE1MzVaFw0yMDAzMTIxMzE1MzVaMCYxJDAiB\
                               gNVBAMMG2xvZ2luLmRldm9sdXRpb25zLmNvbSBUb2tlbjCCASIwDQYJKoZI\
                               hvcNAQEBBQADggEPADCCAQoCggEBAK6SY8VtJzWYqtZjwt2QMan/ajY5irq\
                               z+17iq6Vq379yDlJXagoUFTzYPMwrwzjGSDvmQ7kZlRxGeRNb8+WrhzQAp6\
                               ZegLwxifuKsl+Nr31AzDWcfIvOne20KcCZDvzFTC4/KjBpk1W0PM6lcNszM\
                               ilBX2tlfN578dsM+BSYdk4cNrmV9j+4fJ5Q1UrlztMCfzmP9qrakSAJzVzM\
                               y+QReaRBYeIYiSBYjEzZIVbVI09UEgfRPA7Q5nvIOLu+vTnNDMuOX4KkjRO\
                               5Cismg4U36kqL8oY/qlog/l/xsKVpFuLp4KR88//e/mQwd8kOwDMCgLMORF\
                               tnDKrHgKd3/pH9efcCAwEAAaN8MHowCQYDVR0TBAIwADAOBgNVHQ8BAf8EB\
                               AMCBeAwHQYDVR0lBBYwFAYIKwYBBQUHAwEGCCsGAQUFBwMCMB0GA1UdDgQW\
                               BBQQW2Cx8HUpXfFM3B76WzBb/BhCBDAfBgNVHSMEGDAWgBRWAUlOiE4Z3ww\
                               aHgz284/sYB9NaDANBgkqhkiG9w0BAQsFAAOCAQEAkliCiJF9Z/Y57V6Rrn\
                               gHCBBWtqR+N/A+KHQqWxP2MmJiHVBBnZAueVPsvykO+EfbazNEkUoPVKhUd\
                               5NxEmTEMOBu9HUEzlmA5xDjl5xS7fejJIr7pgbxIup4m+DsNsPVnF1Snk56\
                               F6660RhRb9fsHQ0pgvWuG+tQXJ4J1Zi0cp+xi4yze6hJGAyAqj6wU46AUiL\
                               6kUr9GUVHqEsl5mNMIW18JT4KM/s5DWxFGO2soSTkaVHwGSkMBQSTgHMWs0\
                               L3bBfimjw9FwjwwHAbe1W5QU6uVXGApuKANRsXxgCn566QkE/BuV3WVR6uy\
                               n2P1J/vU9hxasgRIcjf3jHC4lGpew==";

    const X509_CERT_1: &str = "MIIDRjCCAi6gAwIBAgIUUqhc3/U6OhKtEk1b8JfX3GL0FPYwDQYJKoZIhvc\
                               NAQELBQAwKDEmMCQGA1UEAwwdbG9naW4uZGV2b2x1dGlvbnMuY29tIFJvb3\
                               QgQ0EwHhcNMTkwMzEzMTMxNTM1WhcNMjAwMzEyMTMxNTM1WjAqMSgwJgYDV\
                               QQDDB9sb2dpbi5kZXZvbHV0aW9ucy5jb20gQXV0aG9yaXR5MIIBIjANBgkq\
                               hkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAlbRwXVPc/WH4t/Yti5qv24pAu8Q\
                               m0eOVvbum23bYtfJDbCSDh7sY/vvQXgIkM8/0C3tFZ3XaXHbyDHAMn6OC+S\
                               Obzs6SjpfKk9s69Yo/aWFl9oRnAK/+dZ0Y6MTdZO1w+PpR81q5QOFMLpWX1\
                               YNdahaZec31sBmsHqlW04OrHUhGOTGdWNots9/PWvN//x++FL+Sqgh/jxF7\
                               khbgfAuz1QKa8P0ZlE4cOcRIs5bSnUFwtoytKH02/YZnCJD7I/iXFuCPV/+\
                               LZO6yobkTREE3npeXvAKr1OKF2F0JVORMhHiYyguh9t3bMwHTCFqmfQkIMD\
                               GjaTJD7bd8y2Au+eDzgwIDAQABo2YwZDAOBgNVHQ8BAf8EBAMCAQYwEgYDV\
                               R0TAQH/BAgwBgEB/wIBAjAdBgNVHQ4EFgQUVgFJTohOGd8MGh4M9vOP7GAf\
                               TWgwHwYDVR0jBBgwFoAU42BA1coGHUUPUSeacQfTzicjosgwDQYJKoZIhvc\
                               NAQELBQADggEBAKyyDs+uIughmloEmlf8s1cSP8cLtC1Di2TfYSG0bpEM3B\
                               EPTond/7ujDlv0eug9NRurvWd5v7bWvy9VlJo+x2rLBmkzaNcBSVHZ4UbFU\
                               90MSvHjxNZ7VbUfbWsJVeaYHtqf1m3z0fYT0tUor3chD+wbSqraWw4+t54h\
                               fJl22jExTWS9X0F5/Gf3LQOiOvtjHP+b3VkpXkEPIBbvIO/X6kgoGDLm/lA\
                               IPdZmpI956z5+acLHu3AQkxNXQPzCjSSdJphLVU1XeHXOMWldVtE9BqSMVI\
                               HZ6oCz/FtMA4F6R7WiVXXGR+ywRwFyeiFoRea2ImUK9TRWFsaXKeOBMm+TL\
                               bk=";

    const X509_CERT_2: &str = "MIIDRDCCAiygAwIBAgIUCAKwhsjTttdG4koEAV7zqlnI7wkwDQYJKoZIhvc\
                               NAQELBQAwKDEmMCQGA1UEAwwdbG9naW4uZGV2b2x1dGlvbnMuY29tIFJvb3\
                               QgQ0EwHhcNMTkwMzEzMTMxNTM1WhcNMjQwMzExMTMxNTM1WjAoMSYwJAYDV\
                               QQDDB1sb2dpbi5kZXZvbHV0aW9ucy5jb20gUm9vdCBDQTCCASIwDQYJKoZI\
                               hvcNAQEBBQADggEPADCCAQoCggEBANRZxxg9eTCMVr4DsIUcytQOLnlZ7tl\
                               uliP+jM76mjJEuWqizHzZ1ZoPcEbdW9sV8kgWdPHL3KOlXAr0DEobnhQsNx\
                               uzJ8B73TcV7AKp2HR+xCTKPEha1gVHgQMmzQyCIgLEsdcjhsFeFYqMflELZ\
                               rMy+7DBSZWWf3wCnxiKbzTL01wKqylVWeSiXsniTpsoUSSk8Fe2/Li8dBMY\
                               he1vTb57GI8ta24P4lfJv6CPTNTVsr+6ue3lRuY/UIMNTybhBSc00qbuo0K\
                               ahWHyzDgY+iNEaALbyWeNOoTBQIO8lp4mhHcO/Znh2PxdqCi/FSCB2+A1Xd\
                               uOArn+MKegU5aVJN0CAwEAAaNmMGQwEgYDVR0TAQH/BAgwBgEB/wIBAjAOB\
                               gNVHQ8BAf8EBAMCAQYwHQYDVR0OBBYEFONgQNXKBh1FD1EnmnEH084nI6LI\
                               MB8GA1UdIwQYMBaAFONgQNXKBh1FD1EnmnEH084nI6LIMA0GCSqGSIb3DQE\
                               BCwUAA4IBAQB+v34Vk/+qQgA7eWlczWNVWM0J67om+QwtMEo+VgzE2OHNID\
                               2o5QXsxcck0j8dANutkoqsUXpos/RG+QPNng5RBWA/sWUYWdfwZgrE30rBK\
                               waP8Yi8gVsZpz3/RClbPcfkUXI12ANw3bRI1TscOK165p1TV6nmeEus5LZq\
                               CJV37/WRt47CccsDNZaqSN7T5lQ045jsZVYpfgx/I1l9Q/fICrTOFwqYbXJ\
                               9DTe1v8C+LFbtTNcEzRGwZefLTNH2yuZjGy1/t4+cnmFJUzmC4abOoZcpkr\
                               z6U68caCbQA+wdmFs4XaO2bFaiyM+m0LVMOQfLuX/0RZc2KB7fAbb7oHQl";

    fn get_jwk_set() -> JwkSet {
        JwkSet {
            keys: vec![Jwk {
                algorithm: Some(SignatureHashType::RsaSha256),
                key_operations: Some(vec![JwkKeyOps::Verify]),
                key_id: Some("bG9naW4uZGV2b2x1dGlvbnMuY29tIFRva2VuLk1hciAxMyAxMzoxNTozNSAyMDE5IEdNVA".to_owned()),
                x509_sha1_thumbprint: Some(X509_SHA1_THUMBPRINT.to_owned()),
                x509_chain: Some(vec![
                    X509_CERT_0.to_owned(),
                    X509_CERT_1.to_owned(),
                    X509_CERT_2.to_owned(),
                ]),
                ..Jwk::new(JwkKeyType::new_rsa_key_from_base64_url(
                    RSA_MODULUS.into(),
                    RSA_PUBLIC_EXPONENT.into(),
                ))
            }],
        }
    }

    #[test]
    fn rsa_key() {
        let expected = get_jwk_set();
        let decoded = JwkSet::from_json(crate::test_files::JOSE_JWK_SET).unwrap();
        pretty_assertions::assert_eq!(decoded, expected);

        let encoded = expected.to_json_pretty().unwrap();
        let decoded = JwkSet::from_json(&encoded).unwrap();
        pretty_assertions::assert_eq!(decoded, expected);
    }
}
