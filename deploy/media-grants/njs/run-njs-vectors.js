// Usage: njs -n <njs|QuickJS> -m -p <dir of this file> run-njs-vectors.js <vectors-v1.json>
import fs from "fs";
import grants from "./media-grant.js";
import checks from "./vector-checks.js";

const corpusPath = process.argv[process.argv.length - 1];
const corpus = JSON.parse(fs.readFileSync(corpusPath, "utf8"));

checks.checkVectors(corpus, grants).then((report) => {
  report.failures.forEach((failure) => console.log(failure));
  if (report.failures.length > 0) {
    throw new Error(`${report.failures.length} media grant vector failures`);
  }
  console.log(`media grant vectors: ${report.cases} cases passed`);
});
