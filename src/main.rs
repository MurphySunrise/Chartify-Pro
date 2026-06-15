#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::collections::{HashMap, HashSet};
use std::env;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use statrs::distribution::{ContinuousCDF, StudentsT};

mod chart;
mod gui;
mod pptx;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Structure {
    Single,
    Multiple,
}

#[derive(Clone, Debug)]
struct Args {
    input: PathBuf,
    output: Option<PathBuf>,
    structure: Structure,
    group_col: String,
    item_col: Option<String>,
    data_cols: Vec<String>,
    control_group: String,
    charts_dir: Option<PathBuf>,
    sample_size: usize,
    ppt_template: Option<PathBuf>,
    ppt_output: Option<PathBuf>,
}

#[derive(Debug)]
struct LongRow {
    group: String,
    item: String,
    value: f64,
}

#[derive(Debug)]
struct StatRow {
    item: String,
    group: String,
    count: usize,
    mean: Option<f64>,
    median: Option<f64>,
    q5: Option<f64>,
    q95: Option<f64>,
    std: Option<f64>,
    sigma_delta: Option<f64>,
    p_value: Option<f64>,
}

fn main() {
    let result = if env::args().len() == 1 {
        gui::launch()
    } else {
        run_cli()
    };
    if let Err(error) = result {
        eprintln!("Error: {error:#}");
        std::process::exit(1);
    }
}

fn run_cli() -> Result<()> {
    let args = parse_args(env::args().skip(1))?;
    let summary = process(&args)?;

    if let Some(output) = &args.output {
        println!(
            "Generated {} statistic rows at {}",
            summary.stat_count,
            output.display()
        );
    }
    if let Some(charts_dir) = &args.charts_dir {
        println!(
            "Generated {} chart images at {}",
            summary.chart_count,
            charts_dir.display()
        );
    }
    if let Some(ppt_output) = &summary.ppt_output {
        println!("Generated PowerPoint report at {}", ppt_output.display());
    }
    if summary.invalid_values > 0 {
        println!(
            "Ignored {} empty or non-numeric data values",
            summary.invalid_values
        );
    }
    Ok(())
}

struct ProcessSummary {
    stat_count: usize,
    chart_count: usize,
    invalid_values: usize,
    ppt_output: Option<PathBuf>,
}

fn process(args: &Args) -> Result<ProcessSummary> {
    let rows = read_csv(&args)?;
    let stats = calculate_stats(
        &rows.long_rows,
        &rows.items,
        &rows.groups,
        &args.control_group,
    )?;
    if let Some(output) = &args.output {
        write_stats(output, &stats)?;
    }
    let temporary_charts = if args.ppt_template.is_some() && args.charts_dir.is_none() {
        Some(tempfile::tempdir().context("Unable to create chart workspace")?)
    } else {
        None
    };
    let charts_dir = args
        .charts_dir
        .as_deref()
        .or_else(|| temporary_charts.as_ref().map(|directory| directory.path()));
    let chart_images = if let Some(charts_dir) = charts_dir {
        chart::render_charts(
            charts_dir,
            &rows.items,
            &rows.groups,
            &rows.long_rows,
            &stats,
            args.sample_size,
        )?
    } else {
        Vec::new()
    };
    let ppt_output = match (&args.ppt_template, &args.ppt_output) {
        (Some(template), Some(output)) => {
            pptx::create_report(template, output, &chart_images)?;
            Some(output.clone())
        }
        (Some(_), None) => bail!("PPT output path is required when a template is selected"),
        (None, Some(_)) => bail!("PPT template is required when a PPT output is selected"),
        (None, None) => None,
    };
    Ok(ProcessSummary {
        stat_count: stats.len(),
        chart_count: chart_images.len(),
        invalid_values: rows.invalid_values,
        ppt_output,
    })
}

fn print_help() {
    println!(
        "\
Chartify Pro

Usage:
  chartify-pro --input INPUT.csv --output OUTPUT.csv \\
    --structure single --group-col GROUP --item-col ITEM \\
    --data-cols VALUE --control-group CONTROL

  chartify-pro --input INPUT.csv --output OUTPUT.csv \\
    --structure multiple --group-col GROUP \\
    --data-cols METRIC_A,METRIC_B --control-group CONTROL

Options:
  --input PATH
  --output PATH
  --structure single|multiple
  --group-col NAME
  --item-col NAME          Required for single structure
  --data-cols NAME,...     One column for single, one or more for multiple
  --control-group VALUE
  --charts-dir PATH        Optional directory for PNG charts
  --sample-size NUMBER     Maximum plotted values per group (default: 10000)
  --ppt-template PATH      Optional PowerPoint template
  --ppt-output PATH        Output PPTX path
  -h, --help
"
    );
}

fn parse_args<I>(args: I) -> Result<Args>
where
    I: IntoIterator<Item = String>,
{
    let mut values = HashMap::new();
    let mut iterator = args.into_iter();

    while let Some(flag) = iterator.next() {
        if flag == "-h" || flag == "--help" {
            print_help();
            std::process::exit(0);
        }
        if !flag.starts_with("--") {
            bail!("Unexpected argument: {flag}");
        }
        let value = iterator
            .next()
            .with_context(|| format!("Missing value for {flag}"))?;
        values.insert(flag, value);
    }

    let input = required(&values, "--input")?.into();
    let output = values.get("--output").map(PathBuf::from);
    let group_col = required(&values, "--group-col")?.to_owned();
    let control_group = required(&values, "--control-group")?.to_owned();
    let structure = match required(&values, "--structure")? {
        "single" => Structure::Single,
        "multiple" => Structure::Multiple,
        other => bail!("Unknown structure '{other}'; use single or multiple"),
    };
    let item_col = values.get("--item-col").cloned();
    let charts_dir = values.get("--charts-dir").map(PathBuf::from);
    let ppt_template = values.get("--ppt-template").map(PathBuf::from);
    let ppt_output = values.get("--ppt-output").map(PathBuf::from);
    let sample_size = values
        .get("--sample-size")
        .map(|value| {
            value
                .parse::<usize>()
                .with_context(|| format!("Invalid --sample-size value '{value}'"))
        })
        .transpose()?
        .unwrap_or(10_000);
    let data_cols = required(&values, "--data-cols")?
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();

    if data_cols.is_empty() {
        bail!("--data-cols must contain at least one column");
    }
    if sample_size == 0 {
        bail!("--sample-size must be greater than zero");
    }
    if structure == Structure::Single {
        if item_col.is_none() {
            bail!("--item-col is required for single structure");
        }
        if data_cols.len() != 1 {
            bail!("single structure requires exactly one data column");
        }
    }

    Ok(Args {
        input,
        output,
        structure,
        group_col,
        item_col,
        data_cols,
        control_group,
        charts_dir,
        sample_size,
        ppt_template,
        ppt_output,
    })
}

fn required<'a>(values: &'a HashMap<String, String>, key: &str) -> Result<&'a str> {
    values
        .get(key)
        .map(String::as_str)
        .with_context(|| format!("Missing required option {key}"))
}

struct InputRows {
    long_rows: Vec<LongRow>,
    items: Vec<String>,
    groups: Vec<String>,
    invalid_values: usize,
}

fn read_csv(args: &Args) -> Result<InputRows> {
    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .from_path(&args.input)
        .with_context(|| format!("Unable to open {}", args.input.display()))?;
    let headers = reader.headers()?.clone();

    let group_index = column_index(&headers, &args.group_col)?;
    let item_index = args
        .item_col
        .as_deref()
        .map(|name| column_index(&headers, name))
        .transpose()?;
    let data_indices = args
        .data_cols
        .iter()
        .map(|name| Ok((name.clone(), column_index(&headers, name)?)))
        .collect::<Result<Vec<_>>>()?;

    let mut long_rows = Vec::new();
    let mut items = Vec::new();
    let mut groups = Vec::new();
    let mut seen_items = HashSet::new();
    let mut seen_groups = HashSet::new();
    let mut invalid_values = 0;

    if args.structure == Structure::Multiple {
        for item in &args.data_cols {
            push_unique(&mut items, &mut seen_items, item.clone());
        }
    }

    for record in reader.records() {
        let record = record?;
        let group = record.get(group_index).unwrap_or("").trim().to_owned();
        if group.is_empty() {
            bail!("Group column '{}' contains an empty value", args.group_col);
        }
        push_unique(&mut groups, &mut seen_groups, group.clone());

        match args.structure {
            Structure::Single => {
                let item = record
                    .get(item_index.expect("validated item column"))
                    .unwrap_or("")
                    .trim()
                    .to_owned();
                if item.is_empty() {
                    bail!(
                        "Item column '{}' contains an empty value",
                        args.item_col.as_deref().unwrap_or("")
                    );
                }
                push_unique(&mut items, &mut seen_items, item.clone());
                match parse_number(record.get(data_indices[0].1).unwrap_or("")) {
                    Some(value) => long_rows.push(LongRow { group, item, value }),
                    None => invalid_values += 1,
                }
            }
            Structure::Multiple => {
                for (item, index) in &data_indices {
                    match parse_number(record.get(*index).unwrap_or("")) {
                        Some(value) => long_rows.push(LongRow {
                            group: group.clone(),
                            item: item.clone(),
                            value,
                        }),
                        None => invalid_values += 1,
                    }
                }
            }
        }
    }

    if groups.is_empty() {
        bail!("CSV contains no data rows");
    }
    if !seen_groups.contains(&args.control_group) {
        bail!(
            "Control group '{}' does not exist in column '{}'",
            args.control_group,
            args.group_col
        );
    }

    Ok(InputRows {
        long_rows,
        items,
        groups,
        invalid_values,
    })
}

fn column_index(headers: &csv::StringRecord, name: &str) -> Result<usize> {
    headers
        .iter()
        .position(|header| header == name)
        .with_context(|| format!("Column '{name}' was not found"))
}

fn parse_number(value: &str) -> Option<f64> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    value
        .parse::<f64>()
        .ok()
        .filter(|number| number.is_finite())
}

fn push_unique(values: &mut Vec<String>, seen: &mut HashSet<String>, value: String) {
    if seen.insert(value.clone()) {
        values.push(value);
    }
}

fn calculate_stats(
    long_rows: &[LongRow],
    items: &[String],
    groups: &[String],
    control_group: &str,
) -> Result<Vec<StatRow>> {
    let mut values_by_key: HashMap<(&str, &str), Vec<f64>> = HashMap::new();
    for row in long_rows {
        values_by_key
            .entry((&row.item, &row.group))
            .or_default()
            .push(row.value);
    }

    let mut rows = Vec::with_capacity(items.len() * groups.len());
    for item in items {
        let control_values = values_by_key
            .get(&(item.as_str(), control_group))
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let control_mean = mean(control_values);
        let control_std = sample_std(control_values);

        for group in groups {
            let values = values_by_key
                .get(&(item.as_str(), group.as_str()))
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let current_mean = mean(values);
            let sigma_delta = if group == control_group {
                None
            } else {
                match (current_mean, control_mean, control_std) {
                    (Some(current), Some(control), Some(std)) if std != 0.0 => {
                        Some((current - control) / std)
                    }
                    _ => None,
                }
            };
            let p_value = if group == control_group {
                None
            } else {
                welch_p_value(values, control_values)?
            };

            rows.push(StatRow {
                item: item.clone(),
                group: group.clone(),
                count: values.len(),
                mean: current_mean,
                median: median(values),
                q5: quantile_nearest(values, 0.05),
                q95: quantile_nearest(values, 0.95),
                std: sample_std(values),
                sigma_delta,
                p_value,
            });
        }
    }
    Ok(rows)
}

fn mean(values: &[f64]) -> Option<f64> {
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}

fn sample_std(values: &[f64]) -> Option<f64> {
    if values.len() < 2 {
        return None;
    }
    let average = mean(values)?;
    let variance = values
        .iter()
        .map(|value| (value - average).powi(2))
        .sum::<f64>()
        / (values.len() - 1) as f64;
    Some(variance.sqrt())
}

fn median(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let middle = sorted.len() / 2;
    if sorted.len() % 2 == 0 {
        Some((sorted[middle - 1] + sorted[middle]) / 2.0)
    } else {
        Some(sorted[middle])
    }
}

fn quantile_nearest(values: &[f64], probability: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let index = ((sorted.len() - 1) as f64 * probability).round() as usize;
    sorted.get(index).copied()
}

fn welch_p_value(left: &[f64], right: &[f64]) -> Result<Option<f64>> {
    if left.len() < 2 || right.len() < 2 {
        return Ok(None);
    }

    let left_mean = mean(left).expect("non-empty values");
    let right_mean = mean(right).expect("non-empty values");
    let left_std = sample_std(left).expect("at least two values");
    let right_std = sample_std(right).expect("at least two values");
    let left_term = left_std.powi(2) / left.len() as f64;
    let right_term = right_std.powi(2) / right.len() as f64;
    let denominator = (left_term + right_term).sqrt();

    if denominator == 0.0 {
        return Ok(None);
    }

    let degrees_of_freedom = (left_term + right_term).powi(2)
        / (left_term.powi(2) / (left.len() - 1) as f64
            + right_term.powi(2) / (right.len() - 1) as f64);
    if !degrees_of_freedom.is_finite() || degrees_of_freedom <= 0.0 {
        return Ok(None);
    }

    let t_value = (left_mean - right_mean) / denominator;
    let distribution = StudentsT::new(0.0, 1.0, degrees_of_freedom)
        .context("Unable to construct Student's t distribution")?;
    Ok(Some(2.0 * distribution.sf(t_value.abs())))
}

fn write_stats(path: &PathBuf, rows: &[StatRow]) -> Result<()> {
    let mut writer = csv::Writer::from_path(path)
        .with_context(|| format!("Unable to create {}", path.display()))?;
    writer.write_record([
        "item",
        "group",
        "count",
        "mean",
        "median",
        "q5",
        "q95",
        "std",
        "sigma_delta",
        "p_value",
    ])?;

    for row in rows {
        writer.write_record([
            row.item.clone(),
            row.group.clone(),
            row.count.to_string(),
            format_optional(row.mean),
            format_optional(row.median),
            format_optional(row.q5),
            format_optional(row.q95),
            format_optional(row.std),
            format_optional(row.sigma_delta),
            format_optional(row.p_value),
        ])?;
    }
    writer.flush()?;
    Ok(())
}

fn format_optional(value: Option<f64>) -> String {
    value.map(|number| number.to_string()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_group_is_preserved() {
        let rows = vec![
            LongRow {
                group: "A".into(),
                item: "metric".into(),
                value: 1.0,
            },
            LongRow {
                group: "A".into(),
                item: "metric".into(),
                value: 2.0,
            },
        ];
        let stats =
            calculate_stats(&rows, &["metric".into()], &["A".into(), "B".into()], "A").unwrap();

        assert_eq!(stats.len(), 2);
        assert_eq!(stats[1].count, 0);
        assert!(stats[1].mean.is_none());
        assert!(stats[1].sigma_delta.is_none());
        assert!(stats[1].p_value.is_none());
    }

    #[test]
    fn welch_test_matches_expected_value() {
        let p_value = welch_p_value(&[3.0, 4.0], &[1.0, 2.0]).unwrap().unwrap();
        assert!((p_value - 0.105572809).abs() < 1e-9);
    }

    #[test]
    fn welch_test_preserves_extreme_tail_probability() {
        let left = [100.0, 101.0, 102.0, 103.0];
        let right = [1.0, 2.0, 3.0, 4.0];
        let p_value = welch_p_value(&left, &right).unwrap().unwrap();
        assert!(p_value > 0.0);
        assert!(p_value < 1e-8);
    }

    #[test]
    fn nearest_quantile_matches_polars_mode() {
        let values = [1.0, 2.0, 3.0, 4.0];
        assert_eq!(median(&values), Some(2.5));
        assert_eq!(quantile_nearest(&values, 0.05), Some(1.0));
        assert_eq!(quantile_nearest(&values, 0.95), Some(4.0));
    }
}
