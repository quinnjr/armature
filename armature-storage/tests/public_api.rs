//! Integration tests exercising `armature-storage` from *outside* the crate.
//!
//! Every other test in this crate is an in-crate `#[cfg(test)]` module, which
//! can reach private items and so cannot see export gaps. `MultipartData` and
//! `parse_multipart` were both unreachable from outside the crate --
//! `collect_all()` was `pub` but callers could not name its return type -- and
//! no test noticed, because there was no test compiled as an external crate.

use armature_storage::{
    Bytes, FileValidator, LocalStorage, LocalStorageConfig, Multipart, MultipartConstraints,
    MultipartData, Storage, StorageError, UploadedFile, ValidationError, http, parse_multipart,
    sniff_content_type,
};
use futures::stream;

const BOUNDARY: &str = "X-INTEGRATION-BOUNDARY";

fn content_type_header() -> http::HeaderValue {
    http::HeaderValue::from_str(&format!("multipart/form-data; boundary={BOUNDARY}")).unwrap()
}

/// Build a `multipart/form-data` body from `(name, filename, content)` parts.
fn build_body(parts: &[(&str, Option<&str>, &str)]) -> Bytes {
    let mut body = String::new();
    for (name, filename, content) in parts {
        body.push_str(&format!("--{BOUNDARY}\r\n"));
        match filename {
            Some(f) => body.push_str(&format!(
                "Content-Disposition: form-data; name=\"{name}\"; filename=\"{f}\"\r\n\
                 Content-Type: application/octet-stream\r\n"
            )),
            None => body.push_str(&format!(
                "Content-Disposition: form-data; name=\"{name}\"\r\n"
            )),
        }
        body.push_str("\r\n");
        body.push_str(content);
        body.push_str("\r\n");
    }
    body.push_str(&format!("--{BOUNDARY}--\r\n"));
    Bytes::from(body)
}

fn body_stream(
    parts: &[(&str, Option<&str>, &str)],
) -> impl futures::Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static {
    let body = build_body(parts);
    stream::once(async move { Ok::<_, std::io::Error>(body) })
}

/// `parse_multipart` is the documented HTTP-request helper. It was entirely
/// unreachable: `mod multipart` is private and the re-export did not list it.
#[tokio::test]
async fn parse_multipart_is_reachable_from_outside_the_crate() {
    let multipart = parse_multipart(
        &content_type_header(),
        body_stream(&[("caption", None, "a photo"), ("file", Some("a.txt"), "hi")]),
    )
    .expect("parse_multipart should accept a well-formed content-type");

    // `MultipartData` must be nameable, otherwise `collect_all()` is unusable.
    let data: MultipartData = multipart.collect_all().await.unwrap();

    assert_eq!(data.field("caption"), Some("a photo"));
    assert!(data.has_files());
    assert_eq!(data.file_count(), 1);
    assert_eq!(
        data.file("file").map(|f| f.data().clone()),
        Some(Bytes::from("hi"))
    );
}

#[tokio::test]
async fn multipart_data_take_file_is_reachable() {
    let mut data: MultipartData = Multipart::from_request(
        "multipart/form-data; boundary=X-INTEGRATION-BOUNDARY",
        body_stream(&[("file", Some("a.txt"), "hi")]),
    )
    .unwrap()
    .collect_all()
    .await
    .unwrap();

    let file: UploadedFile = data.take_file("file").expect("file should be present");
    assert_eq!(file.name(), Some("a.txt"));
    assert!(!data.has_files());
}

/// The default entry points must enforce limits for external callers too.
#[tokio::test]
async fn public_entry_points_enforce_default_constraints() {
    let many: Vec<(String, Option<String>, String)> = (0..11)
        .map(|i| (format!("f{i}"), Some(format!("f{i}.txt")), "x".to_string()))
        .collect();
    let parts: Vec<(&str, Option<&str>, &str)> = many
        .iter()
        .map(|(n, f, c)| (n.as_str(), f.as_deref(), c.as_str()))
        .collect();

    let err = parse_multipart(&content_type_header(), body_stream(&parts))
        .unwrap()
        .collect_files()
        .await
        .expect_err("11 files must exceed the default cap of 10");
    assert!(matches!(err, StorageError::Multipart(_)));

    // ...and the explicit escape hatch must opt out.
    let files = Multipart::unconstrained(body_stream(&parts), BOUNDARY)
        .collect_files()
        .await
        .expect("unconstrained must accept them");
    assert_eq!(files.len(), 11);
}

#[tokio::test]
async fn multipart_constraints_builder_is_usable_externally() {
    let constraints = MultipartConstraints::default().max_files(1).max_fields(4);
    let mp = Multipart::with_constraints(
        body_stream(&[("a", Some("a.txt"), "1"), ("b", Some("b.txt"), "2")]),
        BOUNDARY,
        constraints,
    );

    assert!(matches!(
        mp.collect_files().await,
        Err(StorageError::Multipart(_))
    ));
}

#[tokio::test]
async fn storage_trait_is_usable_through_a_trait_object() {
    let dir = tempfile::tempdir().unwrap();
    let storage: std::sync::Arc<dyn Storage> = std::sync::Arc::new(
        LocalStorage::new(LocalStorageConfig::new(dir.path()))
            .await
            .unwrap(),
    );

    storage.put("a/b.txt", Bytes::from("data")).await.unwrap();
    assert_eq!(storage.get("a/b.txt").await.unwrap(), "data");

    // `temporary_url_default` and `default_url_duration` are on the trait, so
    // they are reachable through `Arc<dyn Storage>`; they used to be inherent
    // methods on the concrete S3/GCS types only.
    assert_eq!(
        storage.default_url_duration(),
        armature_storage::DEFAULT_URL_DURATION
    );
    assert!(
        storage
            .temporary_url_default("a/b.txt")
            .await
            .unwrap()
            .is_none()
    );

    // `list` recurses, so the nested key is visible.
    let keys: Vec<String> = storage
        .list(None)
        .await
        .unwrap()
        .into_iter()
        .map(|m| m.key)
        .collect();
    assert_eq!(keys, ["a/b.txt"]);
}

#[tokio::test]
async fn traversal_keys_are_rejected_through_the_public_api() {
    let dir = tempfile::tempdir().unwrap();
    let storage = LocalStorage::with_path(dir.path()).await.unwrap();

    for key in ["../escape.txt", "/etc/passwd", "a/../../b.txt"] {
        let err = storage.put(key, Bytes::from("nope")).await.unwrap_err();
        assert!(
            matches!(err, StorageError::InvalidFileName(_)),
            "expected InvalidFileName for {key:?}, got {err:?}"
        );
    }
}

#[test]
fn validation_surface_is_reachable() {
    let validator = FileValidator::new().max_size(1024).images_only();

    // Declared image/png, actual content a PDF.
    let liar = UploadedFile::from_bytes(Bytes::from_static(b"%PDF-1.7\n%%EOF\n"), "x.png");
    let err = validator.validate(&liar).unwrap_err();
    assert!(matches!(
        err.kind(),
        ValidationError::ContentMismatch { .. } | ValidationError::TypeNotAllowed { .. }
    ));

    // `sniff_content_type` is part of the public surface.
    assert_eq!(
        sniff_content_type(b"%PDF-1.7\n%%EOF\n").as_deref(),
        Some("application/pdf")
    );
}

#[test]
fn custom_rule_names_reach_external_callers() {
    let validator = FileValidator::new().custom("no-empty-uploads", |f| {
        if f.is_empty() {
            Err("empty".to_string())
        } else {
            Ok(())
        }
    });

    let err = validator
        .validate(&UploadedFile::new(Bytes::new()))
        .unwrap_err();
    assert_eq!(err.rule(), Some("no-empty-uploads"));
    assert!(err.to_string().contains("no-empty-uploads"));
}

#[test]
fn prelude_covers_the_common_surface() {
    use armature_storage::prelude::*;

    let _validator: FileValidator = FileValidator::new().not_empty();
    let _constraints: MultipartConstraints = MultipartConstraints::default();
    let _data: MultipartData = MultipartData::new();
    let _config: LocalStorageConfig = LocalStorageConfig::new("./uploads");
    let _bytes: Bytes = Bytes::from_static(b"x");
    let _meta: StorageMetadata = StorageMetadata::new("k", 1);
    let _err: StorageError = StorageError::NotFound("k".into());
}
