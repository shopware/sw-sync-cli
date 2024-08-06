const fs = require("fs");

module.exports = async ({ github, context, core }) => {
  const { ARTIFACT_URL, MAIN_COVERAGE } = process.env;
  const mainCov = MAIN_COVERAGE || "?";

  let coverageData = "";
  try {
    const coverageText = fs.readFileSync("./coverage.json");
    coverageData = JSON.parse(coverageText).data[0];
  } catch (err) {
    core.error("Error while reading or parsing the coverage JSON");
    core.setFailed(err);
    return;
  }

  let totalLines = coverageData.totals.lines;
  let comment =
    "Summary of the total line code coverage for the whole codebase\n";
  comment += "| Total lines | Covered | Skipped | % (pr) | % (main) |\n";
  comment += "|--|--|--|--|--|\n";
  comment += `| ${totalLines.count} | ${totalLines.covered} | ${
    totalLines.count - totalLines.covered
  } | ${totalLines.percent.toFixed(2)} | ${mainCov} |\n`;
  comment += "\n";

  // file details
  comment += "<details>\n";
  comment += "<summary>Summary of each file (click to expand)</summary>\n";
  comment += "\n";
  comment += "| File | Total lines | Covered | Skipped | % |\n";
  comment += "|--|--|--|--|--|\n";
  const partToSkip = "/home/runner/work/sw-sync-cli/sw-sync-cli/";
  coverageData.files.forEach((file) => {
    const totalLines = file.summary.lines;
    comment += `| ${file.filename.replace(partToSkip, "")} | ${
      totalLines.count
    } | ${totalLines.covered} | ${
      totalLines.count - totalLines.covered
    } | ${totalLines.percent.toFixed(2)} |\n`;
  });
  comment += "\n";
  comment += "</details>\n";

  // more details
  comment += "<details>\n";
  comment += "<summary>More details (click to expand)</summary>\n";
  comment += "\n";
  comment += "### Download full HTML report\n";
  comment += `You can download the full HTML report here: [click to download](${ARTIFACT_URL})\n`;
  comment +=
    "Hint: You need to extract it locally and open the `index.html`, there you can see which lines are not covered in each file.\n";
  comment += "\n";
  comment += "### You can also generate these reports locally\n";
  comment +=
    "For that, you need to install [cargo-llvm-cov](https://github.com/taiki-e/cargo-llvm-cov), then you can run:\n";
  comment += "```bash\n";
  comment += "cargo llvm-cov --all-features --no-fail-fast --open\n";
  comment += "```\n";
  comment +=
    "Hint: There are also other ways to see code coverage in Rust. For example with RustRover, you can execute tests with coverage generation directly in the IDE.\n";
  comment += "### Remember\n";
  comment +=
    "Your tests should be meaningful and not just be written to raise the coverage.\n";
  comment +=
    "Coverage is just a tool to detect forgotten code paths you may want to think about, not your instructor to write tests\n";
  comment += "</details>\n";

  if (context?.issue?.number) {
    github.rest.issues.createComment({
      issue_number: context.issue.number,
      owner: context.repo.owner,
      repo: context.repo.repo,
      body: comment,
    });
  } else {
    console.log(comment);
  }
};
