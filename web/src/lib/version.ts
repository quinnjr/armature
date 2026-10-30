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
export const FRAMEWORK_VERSION = readField('version', '0.9.0');

/**
 * Cargo requirement for depending on the facade. Pre-1.0 the minor version is
 * the compatibility boundary, so `0.9.0` becomes `"0.9"`.
 */
export const DEPENDENCY_REQ = FRAMEWORK_VERSION.split('.').slice(0, 2).join('.');

/**
 * The `[dependencies]` line for the framework. The crate is published as
 * `armature-framework` (`armature` on crates.io is someone else's crate), so a
 * bare `armature = "…"` line would pull in an unrelated package.
 */
export const DEPENDENCY_LINE = `armature = { package = "armature-framework", version = "${DEPENDENCY_REQ}" }`;

/** Minimum supported Rust version. */
export const MSRV = readField('rust-version', '1.94.1');

/** Rust edition the framework targets. */
export const RUST_EDITION = readField('edition', '2024');
