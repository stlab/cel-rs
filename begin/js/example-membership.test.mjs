import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { randomUUID } from "node:crypto";
import {
    copyFileSync, mkdirSync, readFileSync, realpathSync, rmSync, statSync, unlinkSync, writeFileSync,
} from "node:fs";
import { join, sep } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const root = fileURLToPath(new URL("../../", import.meta.url));

/**
 * Creates a standalone package using the actual std-only Begin build script and registers cleanup.
 * The package exclusively owns its examples, Cargo.lock, and target directory.
 * Setup copies O(build-script bytes); cleanup visits the owned fixture tree.
 */
function createFixture(context) {
    const target = join(root, "target");
    mkdirSync(target, { recursive: true });
    const fixture = join(target, `begin-membership-${randomUUID()}`);
    mkdirSync(fixture);
    context.after(() => rmSync(fixture, { recursive: true, force: true }));
    mkdirSync(join(fixture, "src"));
    mkdirSync(join(fixture, "examples"));
    copyFileSync(join(root, "begin", "build.rs"), join(fixture, "build.rs"));
    writeFileSync(join(fixture, "Cargo.toml"), `[package]
name = "begin-membership-fixture"
version = "0.0.0"
edition = "2024"
build = "build.rs"

[workspace]

[features]
desktop = []
`);
    writeFileSync(join(fixture, "src", "main.rs"), `include!(concat!(env!("OUT_DIR"), "/example_manifest.rs"));

/// Consumes the actual generated inventory without app or renderer dependencies.
fn main() {
    let _ = EXAMPLES_WITH_SOURCE;
}
`);
    return fixture;
}

/**
 * Builds the fixture and reads its actual Cargo-generated example manifest.
 * Costs Cargo compilation plus O(Cargo JSON bytes + generated-manifest bytes) decoding/reading.
 */
function manifest(fixture) {
    const target = join(fixture, "target");
    const result = spawnSync("cargo", [
        "build", "--offline", "--manifest-path", join(fixture, "Cargo.toml"),
        "--target-dir", target, "--message-format=json",
    ], { cwd: fixture, encoding: "utf8", maxBuffer: 8 * 1024 * 1024 });
    assert.equal(result.status, 0, result.error?.message ?? result.stderr);
    const records = result.stdout.trim().split("\n").map(line => JSON.parse(line));
    const output = records.find(record => record.reason === "build-script-executed");
    assert.ok(output, "Cargo reports the actual build script's output");
    assert.ok(realpathSync(output.out_dir).startsWith(`${realpathSync(target)}${sep}`),
        "generated output belongs to the fixture's target");
    return readFileSync(join(output.out_dir, "example_manifest.rs"), "utf8");
}

/** Cargo tracks owned directory additions/removals without changing the copied build script. */
test("embedded example membership follows directory additions and removals", context => {
    const fixture = createFixture(context);
    const script = join(fixture, "build.rs");
    const scriptMtime = statSync(script, { bigint: true }).mtimeNs;
    const name = "added_example";
    const path = join(fixture, "examples", `${name}.adm2`);
    assert.ok(!manifest(fixture).includes(name));
    writeFileSync(path, "sheet membership_probe {}", { flag: "wx" });
    assert.ok(manifest(fixture).includes(name), "new example is embedded after an ordinary rebuild");
    unlinkSync(path);
    assert.ok(!manifest(fixture).includes(name), "removed example leaves the generated manifest");
    assert.equal(statSync(script, { bigint: true }).mtimeNs, scriptMtime,
        "directory membership alone invalidates the unchanged build script");
});
