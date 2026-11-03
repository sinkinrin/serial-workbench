import { build } from "esbuild";
import { copyFile, mkdir, rename } from "node:fs/promises";
await mkdir("dist", { recursive: true });
await build({
  entryPoints: ["src/app.ts"],
  bundle: true,
  minify: true,
  format: "esm",
  outfile: "dist/app.js",
  legalComments: "eof",
});
await copyFile("src/index.html", "dist/index.html");

await rename("dist/app.css", "dist/style.css");
