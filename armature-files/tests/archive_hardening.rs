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

    let err = ZipExtractor::new(archive)
        .extract_to(&target)
        .await
        .expect_err("extraction must refuse to write through a pre-existing symlink");

    // Assert the *specific* refusal, not merely "some error": a bare
    // `is_err()` is satisfied by a staging-creation failure, a permissions
    // problem, or a corrupt fixture, none of which prove the leaf symlink was
    // the thing that stopped the write.
    assert!(
        err.to_string().contains("refusing to overwrite"),
        "expected the O_EXCL destination-reservation refusal, got: {err}"
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

/// A 22-byte End Of Central Directory record declaring `count` members and no
/// comment. Every byte is below 0x80, so the record doubles as a valid UTF-8
/// string and can be planted verbatim in an archive comment.
fn fake_eocd(count: u16) -> Vec<u8> {
    let mut record = vec![b'P', b'K', 0x05, 0x06];
    record.extend_from_slice(&0u16.to_le_bytes()); // this disk
    record.extend_from_slice(&0u16.to_le_bytes()); // disk with the CD start
    record.extend_from_slice(&count.to_le_bytes()); // entries on this disk
    record.extend_from_slice(&count.to_le_bytes()); // total entries
    record.extend_from_slice(&0u32.to_le_bytes()); // CD size
    record.extend_from_slice(&0u32.to_le_bytes()); // CD offset
    record.extend_from_slice(&0u16.to_le_bytes()); // comment length
    assert_eq!(record.len(), 22);
    record
}

/// Spoof #1: a second EOCD planted *inside* the real one's comment.
///
/// A backwards scan that takes the last `PK\x05\x06` in the tail reads the
/// planted record — which declares one member — and waves a 20-member archive
/// through the pre-parse guard. Both records are individually self-consistent
/// (each one's comment length reaches the end of the file), so the guard has to
/// take the *largest* declared count rather than trusting whichever it finds
/// first.
#[test]
fn planted_eocd_in_the_comment_cannot_understate_the_entry_count() {
    let planted = String::from_utf8(fake_eocd(1)).expect("the fixture record is ASCII-safe");

    let mut builder = ZipBuilder::new().comment(planted);
    for i in 0..20 {
        builder = builder.add_file(format!("f{i}.txt"), "x");
    }
    let archive = builder.build().unwrap();

    let err = ZipExtractor::new(archive.data)
        .max_entries(5)
        .list_files()
        .expect_err("a planted EOCD must not launder 20 entries past a limit of 5");

    assert!(
        err.to_string().contains("declares 20 entries"),
        "the real EOCD should have won the scan, got: {err}"
    );
}

/// Spoof #2: a small classic count paired with a huge Zip64 count.
///
/// `zip` 8.x sizes its central-directory index from the Zip64 record whenever a
/// locator is present, so consulting Zip64 only when the 16-bit field is
/// saturated at `0xFFFF` lets an archive declare `5` classically and a million
/// in Zip64 — ~46 MB on the wire, under the container cap, and a million
/// allocated records once opened.
#[test]
fn zip64_entry_count_is_consulted_even_when_the_classic_count_is_small() {
    let mut data = Vec::new();

    let zip64_offset = data.len() as u64;
    let mut zip64 = Vec::from(b"PK\x06\x06".as_slice());
    zip64.extend_from_slice(&44u64.to_le_bytes()); // size of this record
    zip64.extend_from_slice(&45u16.to_le_bytes()); // version made by
    zip64.extend_from_slice(&45u16.to_le_bytes()); // version needed
    zip64.extend_from_slice(&0u32.to_le_bytes()); // this disk
    zip64.extend_from_slice(&0u32.to_le_bytes()); // disk with the CD start
    zip64.extend_from_slice(&1_000_000u64.to_le_bytes()); // entries on this disk
    zip64.extend_from_slice(&1_000_000u64.to_le_bytes()); // total entries
    zip64.extend_from_slice(&0u64.to_le_bytes()); // CD size
    zip64.extend_from_slice(&0u64.to_le_bytes()); // CD offset
    assert_eq!(zip64.len(), 56);
    data.extend_from_slice(&zip64);

    data.extend_from_slice(b"PK\x06\x07"); // Zip64 EOCD locator
    data.extend_from_slice(&0u32.to_le_bytes()); // disk with the Zip64 EOCD
    data.extend_from_slice(&zip64_offset.to_le_bytes());
    data.extend_from_slice(&1u32.to_le_bytes()); // total disks

    // The classic record declares a harmless five.
    data.extend_from_slice(&fake_eocd(5));

    let err = ZipExtractor::new(Bytes::from(data))
        .max_entries(10)
        .list_files()
        .expect_err("the Zip64 count must be enforced even though the classic one fits");

    assert!(
        err.to_string().contains("declares 1000000 entries"),
        "the Zip64 record should have been consulted unconditionally, got: {err}"
    );
}

/// The entry limit is re-applied on every call, not frozen at the parse that
/// populated the index cache.
///
/// `max_entries` takes `self` by value, so the `Arc`'d cache travels with the
/// builder: warming the cache and *then* tightening the limit used to hand back
/// an extractor whose new limit was never consulted again.
#[test]
fn a_tightened_entry_limit_applies_to_an_already_cached_index() {
    let mut builder = ZipBuilder::new();
    for i in 0..20 {
        builder = builder.add_file(format!("f{i}.txt"), "x");
    }
    let archive = builder.build().unwrap();

    let extractor = ZipExtractor::new(archive.data);
    assert_eq!(
        extractor.list_files().unwrap().len(),
        20,
        "the default limit admits 20 entries and warms the index cache"
    );

    let err = extractor
        .max_entries(5)
        .list_files()
        .expect_err("the tightened limit must apply to the cached index too");

    assert!(
        err.to_string().contains("exceeding the limit of 5"),
        "unexpected error: {err}"
    );
}

/// Promotion is all-or-nothing: a failure partway through moving the staged
/// entries into place must not leave the earlier ones behind.
///
/// `sub` is pre-created as a regular *file*, so the second entry's parent
/// directory cannot be created. With `create_dir_all` + `rename` interleaved
/// per entry, `x.txt` was already renamed into the target by the time
/// `sub/y.txt` hit `ENOTDIR` — an `Err` return with attacker-controlled data on
/// the caller's disk, repeatable to fill it.
#[tokio::test]
async fn a_failure_while_promoting_rolls_back_the_entries_already_moved() {
    let archive = ZipBuilder::new()
        .add_file("x.txt", "first")
        .add_file("sub/y.txt", "second")
        .build()
        .unwrap();

    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("out");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("sub"), b"i am a file, not a directory").unwrap();

    ZipExtractor::new(archive.data)
        .extract_to(&target)
        .await
        .expect_err("a non-directory in the way of an entry's parent must fail the extraction");

    assert!(
        !target.join("x.txt").exists(),
        "the first entry was promoted and left behind after the extraction failed"
    );
    assert_eq!(
        std::fs::read(target.join("sub")).unwrap(),
        b"i am a file, not a directory",
        "the pre-existing path must be untouched"
    );

    let leftovers: Vec<_> = std::fs::read_dir(&target)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(
        leftovers.len(),
        1,
        "only the pre-existing `sub` should remain, found {leftovers:?}"
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
