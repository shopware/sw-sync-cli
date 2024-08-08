const fs = require("fs");

module.exports = ({ core }) => {
  try {
    const coverageText = fs.readFileSync("./main-coverage/coverage.json");
    coverageData = JSON.parse(coverageText).data[0];
  } catch (err) {
    core.error("Error while reading or parsing the coverage JSON");
    core.setFailed(err);
    return;
  }

  const totalCoverage = coverageData.totals.lines.percent.toFixed(2);

  core.info(`Total main branch coverage: ${totalCoverage}`);
  core.setOutput("total_coverage", totalCoverage);
};
