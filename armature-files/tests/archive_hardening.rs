//! Hardening regression tests for `armature_files::archive`.
//!
//! Covers the two standard archive attacks: path traversal ("Zip Slip") and
//! decompression bombs. Fixtures are built in-memory so these run anywhere.

#![cfg(feature = "archives")]

use armature_files::archive::{CompressionLevel, ZipBuilder, ZipExtractor};
use bytes::Bytes;
use std::io::Write;

/// Build a ZIP containing the given (name, contents) pairs *verbatim* — the
/// `zip` crate happily writes traversal and absolute member names, which is
/// exactly what a malicious archive looks like on the wire.
fn raw_zip(entries: &[(&str, &[u8])]) -> Bytes {
    let mut buffer = std::io::Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut buffer);
        let options = zip::write::SimpleFileOptions::default();
        for (name, data) in entries {
            writer
                .start_file(*name, options)
                .expect("zip writer should accept the raw member name");
            writer.write_all(data).unwrap();
        }
        writer.finish().unwrap();
    }
    Bytes::from(buffer.into_inner())
}

/// A traversal member (`../`) must be rejected outright, and nothing may be
/// written outside the extraction directory.
#[tokio::test]
async fn extract_to_rejects_parent_traversal_entries() {
    let archive = raw_zip(&[("../escaped.txt", b"pwned")]);

    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("extract_here");
    let sibling = root.path().join("escaped.txt");

    let err = ZipExtractor::new(archive)
        .extract_to(&target)
        .await
        .expect_err("a `../` member must not be extracted");

    assert!(
        err.to_string().contains("escapes the extraction directory"),
        "unexpected error: {err}"
    );
    assert!(
        !sibling.exists(),
        "the traversal entry escaped to {}",
        sibling.display()
    );
}

/// An absolute member name must be neutralized: `Path::join` with an absolute
/// path discards the base entirely, so unguarded this writes straight to the
/// filesystem root. The entry must land *under* the extraction directory (or
/// be rejected) — never at the absolute location it names.
/// The absolute path is derived from this test's *own* temporary directory
/// rather than a fixed `/tmp` name. A constant path is self-poisoning: run once
/// against unfixed code — precisely the scenario this test guards — and it
/// creates the file it asserts the absence of, so every later run fails until
/// someone deletes it by hand. It also collides between concurrent CI
/// containers sharing `/tmp`, and is meaningless on Windows.
#[tokio::test]
async fn extract_to_neutralizes_absolute_entry_paths() {
    let root = tempfile::tempdir().unwrap();
    let absolute = root.path().join("zip-slip-should-not-exist.txt");
    let archive = raw_zip(&[(&absolute.to_string_lossy(), b"pwned")]);

    let target = root.path().join("out");

    // Rejecting the entry outright and safely re-rooting it are both
    // acceptable; writing to the named absolute path is not.
    let extracted = ZipExtractor::new(archive)
        .extract_to(&target)
        .await
        .unwrap_or_default();

    assert!(
        !absolute.exists(),
        "the absolute entry escaped to {}",
        absolute.display()
    );

    let canonical_target = target.canonicalize().unwrap_or_else(|_| target.clone());
    for name in &extracted {
        // An entry that was rejected leaves nothing to canonicalize; only
        // check the ones that really were written.
        let Ok(written) = target.join(name).canonicalize() else {
            continue;
        };
        assert!(
            written.starts_with(&canonical_target),
            "{} was written outside the extraction directory",
            written.display()
        );
    }
}

/// A pre-existing **file** symlink at an entry's destination must not be
/// followed.
///
/// The canonicalized-parent re-check only catches escapes through *directory*
/// symlinks: for an entry named `config.yml`, the parent is the extraction root
/// itself, which canonicalizes inside the root and passes. A plain
/// `std::fs::write` then follows the leaf symlink and overwrites whatever it
/// points at. Reachable whenever the extraction directory is reused or shared —
/// including when a first archive plants the link.
#[cfg(unix)]
#[tokio::test]
async fn extract_to_does_not_write_through_a_pre_existing_symlink() {
    let root = tempfile::tempdir().unwrap();

    let outside = root.path().join("victim.yml");
    std::fs::write(&outside, b"original secret").unwrap();

    let target = root.path().join("out");
    std::fs::create_dir_all(&target).unwrap();
    std::os::unix::fs::symlink(&outside, target.join("config.yml")).unwrap();

    let archive = raw_zip(&[("config.yml", b"pwned")]);

    let result = ZipExtractor::new(archive).extract_to(&target).await;

    assert!(
        result.is_err(),
        "extraction must refuse to write through a pre-existing symlink"
    );
    assert_eq!(
        std::fs::read(&outside).unwrap(),
        b"original secret",
        "the symlink target was overwritten through the link"
    );
    assert!(
        target
            .join("config.yml")
            .symlink_metadata()
            .unwrap()
            .is_symlink(),
        "the planted symlink should have been left alone, not replaced"
    );
}

/// A failed extraction must leave nothing behind.
///
/// Writing entry-by-entry straight into the target means a crafted archive of
/// legitimate entries followed by one bomb entry returns `Err` *and* leaves
/// every preceding entry on the caller's disk — attacker-controlled fill with a
/// clean-looking failure. Extraction is staged and only promoted on success.
#[tokio::test]
async fn failed_extraction_leaves_the_target_directory_empty() {
    let archive = ZipBuilder::new()
        .compression(CompressionLevel::Best)
        .add_file("legit-one.txt", vec![b'a'; 4096])
        .add_file("legit-two/nested.txt", vec![b'b'; 4096])
        .add_file("bomb.bin", vec![0u8; 4 * 1024 * 1024])
        .build()
        .unwrap();

    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("out");

    let err = ZipExtractor::new(archive.data)
        // Room for the two legitimate entries, not for the bomb.
        .max_uncompressed_size(64 * 1024)
        .extract_to(&target)
        .await
        .expect_err("the bomb entry must abort the extraction");
    assert!(
        err.to_string().contains("budget"),
        "unexpected error: {err}"
    );

    let leftovers: Vec<_> = std::fs::read_dir(&target)
        .expect("the target directory should still exist")
        .map(|entry| entry.unwrap().file_name())
        .collect();

    assert!(
        leftovers.is_empty(),
        "a failed extraction left {leftovers:?} behind in the target directory"
    );
}

/// Extraction is non-clobbering: an entry whose destination name is already
/// taken is an error, not an overwrite.
#[tokio::test]
async fn extract_to_refuses_to_overwrite_existing_files() {
    let archive = ZipBuilder::new()
        .add_file("notes.txt", "from archive")
        .build()
        .unwrap();

    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("out");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("notes.txt"), b"pre-existing").unwrap();

    let err = ZipExtractor::new(archive.data)
        .extract_to(&target)
        .await
        .expect_err("an existing destination name must not be silently overwritten");
    assert!(
        err.to_string().contains("refusing to overwrite"),
        "unexpected error: {err}"
    );

    assert_eq!(
        std::fs::read(target.join("notes.txt")).unwrap(),
        b"pre-existing"
    );
}

/// An archive larger than the container cap is refused before it is ever
/// opened, because `ZipArchive::new` indexes the whole central directory (one
/// allocation per record) before any entry-count guard could fire.
#[test]
fn oversized_archive_container_is_rejected_before_parsing() {
    let archive = ZipBuilder::new().add_file("a.txt", "x").build().unwrap();

    let err = ZipExtractor::new(archive.data)
        .max_archive_bytes(16)
        .list_files()
        .expect_err("an archive over the container cap must not be opened");

    assert!(
        err.to_string().contains("exceeding the limit"),
        "unexpected error: {err}"
    );
}

/// The entry-count limit is enforced from the End Of Central Directory record,
/// before the central directory itself is indexed.
#[test]
fn entry_count_is_checked_before_the_central_directory_is_indexed() {
    let mut builder = ZipBuilder::new();
    for i in 0..20 {
        builder = builder.add_file(format!("f{i}.txt"), "x");
    }
    let archive = builder.build().unwrap();

    let err = ZipExtractor::new(archive.data)
        .max_entries(5)
        .list_files()
        .expect_err("20 entries must not pass a limit of 5");

    assert!(
        err.to_string().contains("declares 20 entries"),
        "the limit should have been caught from the EOCD record, got: {err}"
    );
}

/// The async in-memory extraction path behaves like the sync one.
#[tokio::test]
async fn extract_all_async_matches_extract_all() {
    let archive = ZipBuilder::new()
        .add_file("one.txt", "first")
        .add_file("two.txt", "second")
        .build()
        .unwrap();

    let extractor = ZipExtractor::new(archive.data);
    // Warm the index cache first, so the async path is exercised against a
    // populated cache rather than re-parsing.
    let sync = extractor.extract_all().unwrap();
    let asynchronous = extractor.extract_all_async().await.unwrap();

    assert_eq!(sync.len(), asynchronous.len());
    for (a, b) in sync.iter().zip(&asynchronous) {
        assert_eq!(a.path, b.path);
        assert_eq!(a.data, b.data);
    }
}

/// The in-memory extraction path applies the same guard.
#[test]
fn extract_all_rejects_traversal_entries() {
    let archive = raw_zip(&[("a/../../b.txt", b"pwned")]);

    let err = ZipExtractor::new(archive)
        .extract_all()
        .expect_err("a traversal member must not be returned");

    assert!(
        err.to_string().contains("escapes the extraction directory"),
        "unexpected error: {err}"
    );
}

/// A high-ratio archive must be refused rather than decompressed into RAM: a
/// few KB of deflate stream expands to 4 MiB here, and unbounded that class of
/// input exhausts the heap.
#[test]
fn extract_all_enforces_the_uncompressed_size_budget() {
    let bomb = ZipBuilder::new()
        .compression(CompressionLevel::Best)
        .add_file("bomb.bin", vec![0u8; 4 * 1024 * 1024])
        .build()
        .unwrap();

    // Sanity check: the *compressed* archive is tiny, so a size check on the
    // input bytes would not catch this.
    assert!(
        bomb.data.len() < 64 * 1024,
        "fixture should be highly compressible, got {} bytes",
        bomb.data.len()
    );

    let err = ZipExtractor::new(bomb.data)
        .max_uncompressed_size(1024)
        .extract_all()
        .expect_err("a 4 MiB expansion must not fit in a 1 KiB budget");

    assert!(
        err.to_string().contains("budget"),
        "unexpected error: {err}"
    );
}

/// The budget is cumulative across entries, not per entry: each entry alone
/// fits, but together they blow the cap.
#[test]
fn uncompressed_size_budget_is_cumulative_across_entries() {
    let archive = ZipBuilder::new()
        .compression(CompressionLevel::Best)
        .add_file("one.bin", vec![7u8; 1024 * 1024])
        .add_file("two.bin", vec![9u8; 1024 * 1024])
        .build()
        .unwrap();

    // 1.5 MiB: the first entry fits, the second must not.
    let err = ZipExtractor::new(archive.data.clone())
        .max_uncompressed_size(3 * 1024 * 1024 / 2)
        .extract_all()
        .expect_err("the second entry should exhaust the shared budget");
    assert!(
        err.to_string().contains("budget"),
        "unexpected error: {err}"
    );

    // With headroom for both, extraction succeeds.
    let entries = ZipExtractor::new(archive.data)
        .max_uncompressed_size(4 * 1024 * 1024)
        .extract_all()
        .expect("both entries fit within a 4 MiB budget");
    assert_eq!(entries.len(), 2);
}

/// Entry-count is capped independently of total size.
#[test]
fn entry_count_is_capped() {
    let mut builder = ZipBuilder::new();
    for i in 0..20 {
        builder = builder.add_file(format!("f{i}.txt"), "x");
    }
    let archive = builder.build().unwrap();

    let err = ZipExtractor::new(archive.data)
        .max_entries(5)
        .list_files()
        .expect_err("20 entries must not pass a limit of 5");

    assert!(
        err.to_string().contains("exceeding the limit"),
        "unexpected error: {err}"
    );
}

/// A well-formed archive still round-trips through the on-disk path, and the
/// nested directory structure is recreated under the target directory.
#[tokio::test]
async fn extract_to_writes_nested_entries_under_the_target() {
    let archive = ZipBuilder::new()
        .add_file("top.txt", "top level")
        .add_file("nested/deep/file.txt", "nested content")
        .build()
        .unwrap();

    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("out");

    let extracted = ZipExtractor::new(archive.data)
        .extract_to(&target)
        .await
        .expect("a well-formed archive should extract");

    assert_eq!(extracted.len(), 2);
    assert_eq!(
        std::fs::read_to_string(target.join("top.txt")).unwrap(),
        "top level"
    );
    assert_eq!(
        std::fs::read_to_string(target.join("nested/deep/file.txt")).unwrap(),
        "nested content"
    );
}
