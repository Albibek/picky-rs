# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [6.0.0-alpha] 2020-08-??

## Added

- Implementation of `Jwe` (JSON Web Encryption) RFC

## Changed

- Separate `SignatureHashType` into two separate enums `HashAlgorithm` and `SignatureAlgorithm`.
- `KeyIdGenError` is now wrapping a `HashAlgorithm`.
- Update `rsa` dependency.
- `HttpSignature` store custom `algorithm` names instead of just ignoring them.
- Major `jose` module changes
  - JOSE header fields are renamed to be identical to RFCs when possible.
  - `Jwt` is now divided into `Jws` (JSON Web Signature) and `Jwe` (JSON Web Encryption)
  - `Jws` provides an API to sign any kind of data (binary). JSON claims are part of `Jwt` only.
  - `Jwe` provides an API to encrypt any kind of data (binary). JSON claims are part of `Jwt` only.

## [5.1.1] 2020-07-13

### Changed

- Better `CaChainError::AuthorityKeyIdMismatch` display.

## [5.1.0] 2020-07-07

### Added

- Add `BufRead`-based API to read PEM streams: `pem::Pem::read_from` and `pem::read_pem`.

### Changed

- Some internal types are moved to a new `picky_asn1_x509` crate but API is unchanged.
- Dependencies clean up.

## [5.0.0] 2020-05-06

### Added

- Add `into_public_key` method to `x509::Cert`.
- Support for the ["algorithm" http signature parameter](https://tools.ietf.org/html/draft-cavage-http-signatures-12#section-2.1.3).

### Changed

- `Jwt` has no generic lifetime parameter anymore.

### Removed

- `Cert::verify` and `Cert::verify_chain` methods (previously deprecated)
- `HttpRequest` trait impl for `http` crate v0.1

### Fixed

- Add missing parameters for AES and SHA object identifiers ([668c06e8d](https://github.com/Devolutions/picky-rs/commit/668c06e8d8e8a0caae8bd13cf81c189bbc2e4918))

## [4.7.0] 2020-04-16

### Added

- Implement `From<UTCDate>` trait on `DateTime<Utc>`.
- Support for leeway in HTTP signature verifier.
- Verifier API to X509 Cert.

### Deprecated

- `Cert::verify` and `Cert::verify_chain` methods in favor of the `Cert::verifier` method.
