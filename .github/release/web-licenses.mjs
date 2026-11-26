// Run from web/ after `pnpm install`:
//
//   pnpm licenses list --prod --json --long | node ../.github/release/web-licenses.mjs
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

const LICENSE_FILE = /^(licen[sc]e|copying|notice)(\.|-|$)/i;

const readStdin = () => readFileSync(0, "utf8");

const packages = Object.entries(JSON.parse(readStdin()))
  .flatMap(([license, list]) => list.map((pkg) => ({ ...pkg, license: pkg.license ?? license })))
  .sort((a, b) => a.name.localeCompare(b.name));

const licenseTexts = (dir) =>
  readdirSync(dir)
    .filter((name) => LICENSE_FILE.test(name))
    .sort()
    .map((name) => readFileSync(join(dir, name), "utf8").trimEnd());

const describe = (pkg) => {
  const texts = pkg.paths.flatMap(licenseTexts);
  if (texts.length === 0 && !pkg.license) {
    throw new Error(`${pkg.name}: no license file and no declared license`);
  }
  const header = [
    "=".repeat(80),
    `${pkg.name} ${pkg.versions.join(", ")} (${pkg.license})`,
    ...(pkg.homepage ? [pkg.homepage] : []),
    "",
  ];
  const body = texts.length > 0 ? [...new Set(texts)] : ["(the package ships no license file)"];
  return [...header, ...body, ""].join("\n");
};

process.stdout.write(
  [
    "THIRD-PARTY NOTICES: console dependencies (npm)\n",
    "These packages are bundled into the web console embedded in the binary.\n",
    ...packages.map(describe),
  ].join("\n"),
);
