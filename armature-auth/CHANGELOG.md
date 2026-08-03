# Changelog — `armature-auth`

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Earlier changes are recorded in the workspace [`CHANGELOG.md`](../CHANGELOG.md).

## [Unreleased]

### Fixed

- **Breaking:** `SamlConfig::allow_idp_initiated` defaults to `false`, and `validate_response_with_request_id` correlates `InResponseTo` and RelayState. The generated RelayState was previously handed to the caller and never checked, leaving SSO login-CSRF and unsolicited-response replay open.
- **Breaking:** `MagicLinkToken::verify` is renamed `is_usable`, and the new `verify_token(candidate)` performs the constant-time secret comparison. The old name checked only expiry and the used flag while the module example presented it as the login check.
- Backup codes use all eight bytes of entropy (64 bits). Half were drawn and discarded, leaving 32 bits on a 2FA bypass credential.
- An unknown username now runs a dummy KDF verification, closing a timing-based user-enumeration oracle.
- `ApiKeyManager` rate-limit state moved to a `DashMap`; every validation previously serialized on one process-wide mutex held across a sweep.

### Changed — `0.1.3` → `0.1.4`

- Migrated onto `armature-core` `0.8`'s `Bytes`-backed request and response types. No behavior change beyond what that migration implies; see [`armature-core/CHANGELOG.md`](../armature-core/CHANGELOG.md).
