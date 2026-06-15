# Chartify Pro

Chartify Pro is a native desktop application for comparing statistical groups
from CSV data and generating presentation-ready PowerPoint reports.

## Features

- Supports single-value and multiple-metric CSV layouts
- Preserves empty item/group combinations without crashing
- Calculates descriptive statistics and Welch's t-test
- Produces box plots and normalized quantile plots
- Builds reports from the supplied PowerPoint template
- Saves the report beside the source CSV
- Runs as a standalone macOS or Windows desktop application

## Use

1. Choose a source CSV file.
2. Choose a PowerPoint template.
3. Select the table structure and map the CSV columns.
4. Select the control group and metrics.
5. Generate the report.

The generated file is named `<source>_Statistic_Report.pptx`.

## Build From Source

Rust 1.92 or newer is recommended.

```bash
cargo run --release
cargo test
```

### macOS application

```bash
./scripts/package-macos.sh
```

The application and ZIP archive are written to `dist/`.

### Windows application

The GitHub Actions workflow builds `Chartify-Pro.exe` on a native Windows
runner. Download the `Chartify-Pro-Windows` artifact from the latest workflow
run.

## Command Line

```bash
cargo run --release -- \
  --input sample.csv \
  --output statistics.csv \
  --structure multiple \
  --group-col group \
  --data-cols metric_a,metric_b \
  --control-group A \
  --charts-dir charts
```

Use `cargo run --release -- --help` for all options.

