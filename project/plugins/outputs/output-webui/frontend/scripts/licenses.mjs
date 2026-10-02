// License notices for all installed dependencies in the exact dependency lock.
// Build tools are included conservatively; some never enter the browser bundle.
import fs from "node:fs";
import path from "node:path";
const root = path.resolve(import.meta.dirname, "..");
const lock = JSON.parse(
  fs.readFileSync(path.join(root, "package-lock.json"), "utf8"),
);
let out =
  "# Third-party license notices\n\nlog-print output-webui uses Vue 3, Element Plus, GridStack, AG Grid Community and Apache ECharts. No AG Grid Enterprise modules are included. Browser assets are bundled locally; these notices accompany distribution.\n\n";
for (const [directory, entry] of Object.entries(lock.packages).sort(
  ([a], [b]) => a.localeCompare(b),
)) {
  if (!directory) continue;
  const folder = path.join(root, directory);
  if (!fs.existsSync(folder)) continue;
  const pkg = JSON.parse(
    fs.readFileSync(path.join(folder, "package.json"), "utf8"),
  );
  out += `## ${pkg.name} ${pkg.version}\n\nLicense: ${typeof pkg.license === "string" ? pkg.license : entry.license || "see notices below"}. ${pkg.repository?.url || pkg.homepage || ""}\n\n`;
  const notices = fs
    .readdirSync(folder)
    .filter(
      (name) =>
        /^(licen[sc]e|notice|copyright)(\.|$)/i.test(name) &&
        fs.statSync(path.join(folder, name)).isFile(),
    );
  for (const notice of notices)
    out += `### ${notice}\n\n\`\`\`text\n${fs.readFileSync(path.join(folder, notice), "utf8").trim()}\n\`\`\`\n\n`;
}
// Normalize whitespace without changing the license text.
out = out.split(/\r?\n/).map((line) => line.trimEnd()).join("\n").trimEnd() + "\n";
fs.writeFileSync(path.join(root, "public", "THIRD_PARTY_LICENSES.md"), out);
