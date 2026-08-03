/**
 * Framework facts read from the workspace manifest at build time.
 *
 * The site previously restated the version ("0.1.0") and status ("98% Feature
 * Complete") as literals in three places — the JSON-LD, `llms.txt` and the copy
 * — and all three had drifted from the crate. Structured data that contradicts
 * the artifact it describes is worse than no structured data: an answer engine
 * will quote it.
 */
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const MANIFEST = resolve(process.cwd(), '..', 'Cargo.toml');

function readField(field: string, fallback: string): string {
  try {
    const manifest = readFileSync(MANIFEST, 'utf-8');
    // Only the [package] table: [workspace.package] above it carries its own
    // `version`, and matching the first hit in the file would read that one.
    const packageTable = manifest.slice(manifest.indexOf('\n[package]'));
    const match = new RegExp(`^${field}\\s*=\\s*"([^"]+)"`, 'm').exec(packageTable);
    return match ? match[1] : fallback;
  } catch {
    return fallback;
  }
}

/** Version of the `armature-framework` facade crate. */
export const FRAMEWORK_VERSION = readField('version', '0.4.0');

/** Minimum supported Rust version. */
export const MSRV = readField('rust-version', '1.94.1');

/** Rust edition the framework targets. */
export const RUST_EDITION = readField('edition', '2024');
