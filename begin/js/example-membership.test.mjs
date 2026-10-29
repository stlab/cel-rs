import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { readFileSync, writeFileSync, unlinkSync } from "node:fs";
import { fileURLToPath } from "node:url";
import test from "node:test";

const root = fileURLToPath(new URL("../../", import.meta.url));

/** Builds the real non-desktop app and returns its generated example manifest. */
function manifest() {
    const result = spawnSync("cargo", [
        "build", "-p", "begin", "--no-default-features", "--message-format=json",
    ], { cwd: root, encoding: "utf8", maxBuffer: 8 * 1024 * 1024 });
    assert.equal(result.status, 0, result.stderr);
    const records = result.stdout.trim().split("\n").map(line => JSON.parse(line));
    const output = records.find(record =>
        record.reason === "build-script-executed" && record.package_id.includes("/begin#"));
    assert.ok(output, "Cargo reports begin's generated directory");
    return readFileSync(`${output.out_dir}/example_manifest.rs`, "utf8");
}

/** Cargo invalidates membership when a fixture is added and removed without touching build.rs. */
test("embedded example membership follows directory additions and removals", () => {
    const name = `better_code_membership_probe_${process.pid}`;
    const path = `${root}/begin/examples/${name}.adm2`;
    assert.ok(!manifest().includes(name));
    try {
        writeFileSync(path, "sheet membership_probe {}", { flag: "wx" });
        assert.ok(manifest().includes(name), "new example is embedded after an ordinary rebuild");
    } finally {
        unlinkSync(path);
    }
    assert.ok(!manifest().includes(name), "removed example leaves the generated manifest");
});
