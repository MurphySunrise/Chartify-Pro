# Chartify Rust migration

## Stage 1: statistics CLI

Status: complete

- Read single-column and multiple-column CSV layouts.
- Preserve every item/group combination, including combinations with no values.
- Calculate count, mean, median, Q5, Q95, sample standard deviation,
  Sigma Delta, and Welch two-sample p-value.
- Export the result as CSV.
- Match the Python implementation within `1e-12` on the included fixtures.

## Stage 2: chart rendering

Status: complete

- Render the box plot, jitter points, normal probability plot, legend, and
  summary table to PNG.
- Match the Python significance highlighting.
- Add deterministic sampling for large groups.
- Run chart generation in a bounded worker pool.

## Stage 3: native desktop UI

Status: complete

- Build the file selectors and controls with `egui`/`eframe`.
- Run processing outside the UI thread.
- Show progress, errors, and the generated report path.

## Stage 4: PPTX output

Status: complete

- Copy a selected template.
- Add titled slides in groups of eight images.
- Keep significant and comparable metrics in separate sections.
- Validate output in Microsoft PowerPoint on Windows.

## Stage 5: Windows packaging

- Build a Windows x86-64 release binary.
- Add application metadata and icon.
- Test on a clean Windows machine without Python installed.
