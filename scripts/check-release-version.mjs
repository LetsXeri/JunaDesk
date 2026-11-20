import { readFileSync } from "node:fs";

const tag = process.env.GITHUB_REF_NAME;
const packageJson = JSON.parse(readFileSync("package.json", "utf8"));
const tauri = JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8"));
const cargo = readFileSync("src-tauri/Cargo.toml", "utf8").match(/^version = "([^"]+)"/m)?.[1];
if (!tag || ![packageJson.version, tauri.version, cargo].every((version) => `v${version}` === tag)) {
  throw new Error("Der Tag muss den Versionen in package.json, Tauri und Cargo entsprechen.");
}
