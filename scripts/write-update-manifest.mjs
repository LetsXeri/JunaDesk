import { readFileSync, statSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const [tag, directory] = process.argv.slice(2);
if (!/^v\d+\.\d+\.\d+(?:-[\w.-]+)?(?:\+[\w.-]+)?$/.test(tag ?? "") || !directory) {
  throw new Error("Expected a version tag and a release directory.");
}

const version = tag.slice(1);
const platforms = {};
for (const [target, asset] of [
  ["darwin-aarch64", `JunaDesk-${tag}-macOS-arm64.app.tar.gz`],
  ["windows-x86_64", `JunaDesk-${tag}-Windows-x64.exe`],
]) {
  if (statSync(join(directory, asset)).size === 0) {
    throw new Error(`Empty update artifact: ${asset}`);
  }
  const signature = readFileSync(join(directory, `${asset}.sig`), "utf8").trim();
  const decoded = Buffer.from(signature, "base64").toString("utf8");
  if (!decoded.split("\n").some(line => line.startsWith("trusted comment: ") &&
    line.trimEnd().split(/\s+/).includes(`version:${version}`))) {
    throw new Error(`Missing signed version in ${asset}.sig`);
  }
  platforms[target] = {
    url: `https://github.com/LetsXeri/JunaDesk/releases/download/${tag}/${asset}`,
    signature,
  };
}

writeFileSync(join(directory, "latest.json"), JSON.stringify({ version, platforms }, null, 2) + "\n");
