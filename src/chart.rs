use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use plotters::coord::Shift;
use plotters::prelude::*;
use plotters::style::text_anchor::{HPos, Pos, VPos};
use rayon::prelude::*;
use statrs::distribution::{ContinuousCDF, Normal};

use super::{LongRow, StatRow, median};

const WIDTH: u32 = 1600;
const HEIGHT: u32 = 1000;
const NORMAL_QUANTILES: [f64; 18] = [
    0.001, 0.005, 0.01, 0.02, 0.05, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 0.95, 0.99, 0.995,
    0.999,
];
const NORMAL_QUANTILE_LABELS: [&str; 18] = [
    "0.001", "0.005", "0.01", "0.02", "0.05", "0.1", "0.2", "0.3", "0.4", "0.5", "0.6", "0.7",
    "0.8", "0.9", "0.95", "0.99", "0.995", "0.999",
];

pub(super) struct ChartImage {
    pub item: String,
    pub path: PathBuf,
    pub significant: bool,
}

pub(super) fn render_charts(
    output_dir: &Path,
    items: &[String],
    groups: &[String],
    long_rows: &[LongRow],
    stats: &[StatRow],
    sample_size: usize,
) -> Result<Vec<ChartImage>> {
    fs::create_dir_all(output_dir)
        .with_context(|| format!("Unable to create {}", output_dir.display()))?;

    let mut values_by_item_group: HashMap<(&str, &str), Vec<f64>> = HashMap::new();
    for row in long_rows {
        values_by_item_group
            .entry((&row.item, &row.group))
            .or_default()
            .push(row.value);
    }

    let mut images = items
        .par_iter()
        .enumerate()
        .map(|(index, item)| -> Result<ChartImage> {
            let item_stats = stats
                .iter()
                .filter(|row| row.item == *item)
                .collect::<Vec<_>>();
            let sampled_values = groups
                .iter()
                .map(|group| {
                    let values = values_by_item_group
                        .get(&(item.as_str(), group.as_str()))
                        .map(Vec::as_slice)
                        .unwrap_or(&[]);
                    sample_evenly(values, sample_size)
                })
                .collect::<Vec<_>>();
            let path = output_dir.join(format!("{index:04}_{}.png", safe_filename(item)));
            render_item_chart(&path, item, groups, &sampled_values, &item_stats)?;
            Ok(ChartImage {
                item: item.clone(),
                path,
                significant: item_stats
                    .iter()
                    .any(|row| row.p_value.is_some_and(|value| value < 0.05)),
            })
        })
        .collect::<Result<Vec<_>>>()?;

    images.sort_by_key(|image| {
        items
            .iter()
            .position(|item| item == &image.item)
            .unwrap_or(usize::MAX)
    });
    Ok(images)
}

fn render_item_chart(
    path: &Path,
    item: &str,
    groups: &[String],
    values_by_group: &[Vec<f64>],
    stats: &[&StatRow],
) -> Result<()> {
    let root = BitMapBackend::new(path, (WIDTH, HEIGHT)).into_drawing_area();
    root.fill(&WHITE)?;
    let significant = stats
        .iter()
        .any(|row| row.p_value.is_some_and(|value| value < 0.05));

    let content = root.margin(20, 35, 24, 24);
    let (top, lower) = content.split_vertically(675);
    let (_, lower) = lower.split_vertically(65);
    let (table_area, _) = lower.split_vertically(135);
    let (box_area, right) = top.split_horizontally(720);
    let (quantile_area, legend_area) = right.split_horizontally(720);

    draw_box_chart(&box_area, item, groups, values_by_group, significant)?;
    draw_normal_chart(&quantile_area, item, values_by_group, significant)?;
    draw_legend(&legend_area, groups)?;
    draw_summary_table(&table_area, stats)?;

    root.present()
        .with_context(|| format!("Unable to write {}", path.display()))?;
    Ok(())
}

fn draw_box_chart<DB: DrawingBackend>(
    area: &DrawingArea<DB, Shift>,
    item: &str,
    groups: &[String],
    values_by_group: &[Vec<f64>],
    significant: bool,
) -> Result<()>
where
    DB::ErrorType: 'static,
{
    let all_values = values_by_group
        .iter()
        .flat_map(|values| values.iter().copied())
        .collect::<Vec<_>>();
    let (y_min, y_max) = value_range(&all_values);
    let x_max = groups.len().max(1) as f64 + 0.5;
    let label_groups = groups.to_vec();

    let mut chart = ChartBuilder::on(area)
        .caption(
            format!("Box Chart for\n{}", wrap_title(item, 22)),
            ("sans-serif", 30).into_font().style(FontStyle::Bold),
        )
        .margin(18)
        .x_label_area_size(72)
        .y_label_area_size(72)
        .build_cartesian_2d(0.5f64..x_max, y_min..y_max)?;

    let axis_style = if significant {
        RED.stroke_width(2)
    } else {
        BLACK.stroke_width(2)
    };
    let x_label_formatter = move |value: &f64| group_label(value, &label_groups);
    let mut mesh = chart.configure_mesh();
    mesh.x_desc("Group")
        .y_desc("Value")
        .x_labels(groups.len().max(1))
        .x_label_formatter(&x_label_formatter)
        .axis_style(axis_style)
        .disable_x_mesh()
        .disable_y_mesh()
        .light_line_style(RGBColor(230, 230, 230))
        .axis_desc_style(("sans-serif", 20).into_font().style(FontStyle::Bold))
        .label_style(("sans-serif", 16));
    mesh.draw()?;
    chart.draw_series(std::iter::once(Rectangle::new(
        [(0.5, y_min), (x_max, y_max)],
        axis_style,
    )))?;

    if all_values.is_empty() {
        chart.draw_series(std::iter::once(Text::new(
            "No valid data",
            (
                groups.len().max(1) as f64 / 2.0 + 0.5,
                (y_min + y_max) / 2.0,
            ),
            ("sans-serif", 24).into_font(),
        )))?;
        return Ok(());
    }

    let colors = palette();
    let mut median_points = Vec::new();
    let max_duplicate_count = maximum_duplicate_count(values_by_group);

    for (index, values) in values_by_group.iter().enumerate() {
        if values.is_empty() {
            continue;
        }
        let x = index as f64 + 1.0;
        let min = values.iter().copied().fold(f64::INFINITY, f64::min);
        let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let q1 = percentile_linear(values, 0.25).unwrap_or(min);
        let q2 = median(values).unwrap_or(min);
        let q3 = percentile_linear(values, 0.75).unwrap_or(max);
        let (lower_whisker, upper_whisker) = box_whiskers(values, q1, q3);
        let color = colors[index % colors.len()];
        median_points.push((x, q2));

        chart.draw_series(std::iter::once(Rectangle::new(
            [(x - 0.22, q1), (x + 0.22, q3)],
            color.mix(0.25).filled(),
        )))?;
        chart.draw_series(std::iter::once(Rectangle::new(
            [(x - 0.22, q1), (x + 0.22, q3)],
            color.stroke_width(2),
        )))?;
        chart.draw_series([
            PathElement::new(vec![(x, lower_whisker), (x, q1)], color.stroke_width(2)),
            PathElement::new(vec![(x, q3), (x, upper_whisker)], color.stroke_width(2)),
            PathElement::new(
                vec![(x - 0.12, lower_whisker), (x + 0.12, lower_whisker)],
                color.stroke_width(2),
            ),
            PathElement::new(
                vec![(x - 0.12, upper_whisker), (x + 0.12, upper_whisker)],
                color.stroke_width(2),
            ),
            PathElement::new(vec![(x - 0.2, q2), (x + 0.2, q2)], color.stroke_width(4)),
        ])?;

        let jittered = deterministic_jitter(values, x, max_duplicate_count);
        chart.draw_series(
            jittered
                .into_iter()
                .map(|point| Circle::new(point, 3, color.mix(0.5).filled())),
        )?;
    }

    chart.draw_series(LineSeries::new(
        median_points,
        BLUE.mix(0.8).stroke_width(2),
    ))?;
    Ok(())
}

fn draw_normal_chart<DB: DrawingBackend>(
    area: &DrawingArea<DB, Shift>,
    item: &str,
    values_by_group: &[Vec<f64>],
    significant: bool,
) -> Result<()>
where
    DB::ErrorType: 'static,
{
    let all_values = values_by_group
        .iter()
        .flat_map(|values| values.iter().copied())
        .collect::<Vec<_>>();
    let (y_min, y_max) = value_range(&all_values);
    let normal = Normal::new(0.0, 1.0)?;
    let point_sets = values_by_group
        .iter()
        .map(|values| normal_probability_points(values, &normal))
        .collect::<Vec<_>>();
    let tick_positions = NORMAL_QUANTILES
        .iter()
        .map(|probability| normal.inverse_cdf(*probability))
        .collect::<Vec<_>>();
    let max_abs_x = point_sets
        .iter()
        .flat_map(|points| points.iter().map(|(x, _)| x.abs()))
        .chain(tick_positions.iter().map(|x| x.abs()))
        .fold(0.0, f64::max)
        .max(3.15);
    let x_limit = max_abs_x * 1.04;

    let mut chart = ChartBuilder::on(area)
        .caption(
            format!("Normal Quantile Chart for\n{}", wrap_title(item, 22)),
            ("sans-serif", 30).into_font().style(FontStyle::Bold),
        )
        .margin(18)
        .x_label_area_size(82)
        .y_label_area_size(72)
        .build_cartesian_2d(-x_limit..x_limit, y_min..y_max)?;

    let axis_style = if significant {
        RED.stroke_width(2)
    } else {
        BLACK.stroke_width(2)
    };
    let mut mesh = chart.configure_mesh();
    mesh.x_desc("Normal Quantile")
        .x_labels(0)
        .axis_style(axis_style)
        .disable_mesh()
        .axis_desc_style(("sans-serif", 20).into_font().style(FontStyle::Bold))
        .y_label_style(("sans-serif", 14));
    mesh.draw()?;

    for (position, label) in tick_positions.iter().zip(NORMAL_QUANTILE_LABELS) {
        chart.draw_series(std::iter::once(PathElement::new(
            vec![(*position, y_min), (*position, y_max)],
            RGBColor(165, 165, 165).stroke_width(1),
        )))?;
        chart.draw_series(std::iter::once(
            EmptyElement::at((*position, y_min))
                + Text::new(
                    label,
                    (0, 24),
                    ("sans-serif", 14)
                        .into_font()
                        .transform(FontTransform::Rotate90)
                        .color(&BLACK)
                        .pos(Pos::new(HPos::Center, VPos::Center)),
                ),
        ))?;
    }

    chart.draw_series(std::iter::once(Rectangle::new(
        [(-x_limit, y_min), (x_limit, y_max)],
        axis_style,
    )))?;

    if all_values.is_empty() {
        chart.draw_series(std::iter::once(Text::new(
            "No valid data",
            (0.0, (y_min + y_max) / 2.0),
            ("sans-serif", 24).into_font(),
        )))?;
        return Ok(());
    }

    let colors = palette();
    for (index, points) in point_sets.into_iter().enumerate() {
        if points.is_empty() {
            continue;
        }
        let color = colors[index % colors.len()];
        chart.draw_series(LineSeries::new(
            points.iter().copied(),
            color.stroke_width(2),
        ))?;
        chart.draw_series(
            points
                .into_iter()
                .map(|point| Circle::new(point, 4, color.mix(0.65).filled())),
        )?;
    }

    Ok(())
}

fn draw_legend<DB: DrawingBackend>(area: &DrawingArea<DB, Shift>, groups: &[String]) -> Result<()>
where
    DB::ErrorType: 'static,
{
    let colors = palette();
    for (index, group) in groups.iter().enumerate() {
        let y = 45 + index as i32 * 34;
        let color = colors[index % colors.len()];
        area.draw(&PathElement::new(
            vec![(8, y), (42, y)],
            color.stroke_width(3),
        ))?;
        area.draw(&Circle::new((25, y), 4, color.filled()))?;
        area.draw(&Text::new(
            group.clone(),
            (50, y),
            ("sans-serif", 18)
                .into_font()
                .color(&BLACK)
                .pos(Pos::new(HPos::Left, VPos::Center)),
        ))?;
    }
    Ok(())
}

fn draw_summary_table<DB: DrawingBackend>(
    area: &DrawingArea<DB, Shift>,
    stats: &[&StatRow],
) -> Result<()>
where
    DB::ErrorType: 'static,
{
    let headers = [
        "Group",
        "Count",
        "Mean",
        "Median",
        "Q5",
        "Q95",
        "Sigma_delta",
        "P_value",
    ];
    let (width, height) = area.dim_in_pixel();
    let rows = stats.len() + 1;
    let row_height = height as i32 / rows.max(1) as i32;
    let col_width = width as i32 / headers.len() as i32;

    for row_index in 0..rows {
        let top = row_index as i32 * row_height;
        let bottom = if row_index + 1 == rows {
            height as i32 - 1
        } else {
            (row_index as i32 + 1) * row_height
        };
        let row_is_significant = row_index > 0
            && stats[row_index - 1]
                .p_value
                .is_some_and(|value| value < 0.05);
        let fill = if row_index == 0 {
            RGBColor(220, 220, 220)
        } else if row_is_significant {
            RGBColor(255, 180, 70)
        } else {
            WHITE
        };

        for col_index in 0..headers.len() {
            let left = col_index as i32 * col_width;
            let right = if col_index + 1 == headers.len() {
                width as i32 - 1
            } else {
                (col_index as i32 + 1) * col_width
            };
            area.draw(&Rectangle::new(
                [(left, top), (right, bottom)],
                fill.filled(),
            ))?;
            area.draw(&Rectangle::new(
                [(left, top), (right, bottom)],
                BLACK.stroke_width(2),
            ))?;

            let text = if row_index == 0 {
                headers[col_index].to_owned()
            } else {
                table_value(stats[row_index - 1], col_index)
            };
            let font = if row_index == 0 {
                ("sans-serif", 26).into_font().style(FontStyle::Bold)
            } else {
                ("sans-serif", 24).into_font()
            };
            area.draw(&Text::new(
                text,
                ((left + right) / 2, (top + bottom) / 2),
                font.color(&BLACK).pos(Pos::new(HPos::Center, VPos::Center)),
            ))?;
        }
    }
    Ok(())
}

fn sample_evenly(values: &[f64], sample_size: usize) -> Vec<f64> {
    if values.len() <= sample_size {
        return values.to_vec();
    }
    (0..sample_size)
        .map(|index| {
            let position = index * values.len() / sample_size;
            values[position.min(values.len() - 1)]
        })
        .collect()
}

fn maximum_duplicate_count(values_by_group: &[Vec<f64>]) -> usize {
    let mut totals: HashMap<u64, usize> = HashMap::new();
    for value in values_by_group.iter().flatten() {
        *totals.entry(value.to_bits()).or_default() += 1;
    }
    totals.values().copied().max().unwrap_or(1)
}

fn deterministic_jitter(
    values: &[f64],
    center: f64,
    max_duplicate_count: usize,
) -> Vec<(f64, f64)> {
    let mut occurrences: HashMap<u64, usize> = HashMap::new();
    let mut totals: HashMap<u64, usize> = HashMap::new();
    for value in values {
        *totals.entry(value.to_bits()).or_default() += 1;
    }

    values
        .iter()
        .map(|value| {
            let key = value.to_bits();
            let total = totals[&key];
            let occurrence = occurrences.entry(key).or_default();
            let offset = if total <= 1 {
                0.0
            } else {
                let jitter_range = 0.22 * total as f64 / max_duplicate_count.max(1) as f64;
                -jitter_range + 2.0 * jitter_range * (*occurrence as f64 / (total - 1) as f64)
            };
            *occurrence += 1;
            (center + offset, *value)
        })
        .collect()
}

fn normal_probability_points(values: &[f64], normal: &Normal) -> Vec<(f64, f64)> {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let count = sorted.len();
    sorted
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            let probability = if count == 1 {
                0.5
            } else if index == 0 {
                1.0 - 0.5f64.powf(1.0 / count as f64)
            } else if index + 1 == count {
                0.5f64.powf(1.0 / count as f64)
            } else {
                (index as f64 + 1.0 - 0.3175) / (count as f64 + 0.365)
            };
            (normal.inverse_cdf(probability), value)
        })
        .collect()
}

fn percentile_linear(values: &[f64], probability: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let position = (sorted.len() - 1) as f64 * probability;
    let lower = position.floor() as usize;
    let upper = position.ceil() as usize;
    let fraction = position - lower as f64;
    Some(sorted[lower] * (1.0 - fraction) + sorted[upper] * fraction)
}

fn box_whiskers(values: &[f64], q1: f64, q3: f64) -> (f64, f64) {
    let iqr = q3 - q1;
    let lower_fence = q1 - 1.5 * iqr;
    let upper_fence = q3 + 1.5 * iqr;
    let lower_whisker = values
        .iter()
        .copied()
        .filter(|value| *value >= lower_fence)
        .min_by(f64::total_cmp)
        .unwrap_or(q1);
    let upper_whisker = values
        .iter()
        .copied()
        .filter(|value| *value <= upper_fence)
        .max_by(f64::total_cmp)
        .unwrap_or(q3);
    (lower_whisker, upper_whisker)
}

fn value_range(values: &[f64]) -> (f64, f64) {
    if values.is_empty() {
        return (0.0, 1.0);
    }
    let min = values.iter().copied().fold(f64::INFINITY, f64::min);
    let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if min == max {
        let padding = min.abs().max(1.0) * 0.1;
        (min - padding, max + padding)
    } else {
        let padding = (max - min) * 0.1;
        (min - padding, max + padding)
    }
}

fn group_label(value: &f64, groups: &[String]) -> String {
    let index = value.round() as isize - 1;
    if index >= 0 {
        groups.get(index as usize).cloned().unwrap_or_default()
    } else {
        String::new()
    }
}

fn table_value(row: &StatRow, column: usize) -> String {
    match column {
        0 => row.group.clone(),
        1 => row.count.to_string(),
        2 => format_number(row.mean),
        3 => format_number(row.median),
        4 => format_number(row.q5),
        5 => format_number(row.q95),
        6 => format_number(row.sigma_delta),
        7 => format_number(row.p_value),
        _ => String::new(),
    }
}

fn format_number(value: Option<f64>) -> String {
    value
        .map(|number| format!("{number:.4}"))
        .unwrap_or_default()
}

fn wrap_title(value: &str, width: usize) -> String {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in value.split_whitespace() {
        if !current.is_empty() && current.len() + word.len() + 1 > width {
            lines.push(current);
            current = String::new();
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        value.to_owned()
    } else {
        lines.join("\n")
    }
}

fn safe_filename(value: &str) -> String {
    let cleaned = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    if cleaned.is_empty() {
        "metric".to_owned()
    } else {
        cleaned
    }
}

fn palette() -> [RGBColor; 7] {
    [
        BLUE,
        RED,
        GREEN,
        RGBColor(128, 0, 128),
        RGBColor(255, 140, 0),
        CYAN,
        MAGENTA,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sampling_is_bounded_and_deterministic() {
        let values = (0..100).map(|value| value as f64).collect::<Vec<_>>();
        let first = sample_evenly(&values, 10);
        let second = sample_evenly(&values, 10);
        assert_eq!(first, second);
        assert_eq!(first.len(), 10);
        assert_eq!(first[0], 0.0);
        assert_eq!(first[9], 90.0);
    }

    #[test]
    fn filenames_are_safe() {
        assert_eq!(safe_filename("Metric A/B"), "Metric_A_B");
    }

    #[test]
    fn sparse_duplicate_points_stay_centered() {
        let points = deterministic_jitter(&[5.0, 5.0], 1.0, 10);
        assert_eq!(points, vec![(0.956, 5.0), (1.044, 5.0)]);
    }

    #[test]
    fn box_whiskers_exclude_outliers_using_matplotlib_rule() {
        let values = [0.0, 10.0, 11.0, 12.0, 13.0, 14.0, 100.0];
        let q1 = percentile_linear(&values, 0.25).unwrap();
        let q3 = percentile_linear(&values, 0.75).unwrap();

        assert_eq!(box_whiskers(&values, q1, q3), (10.0, 14.0));
    }

    #[test]
    fn normal_probability_points_match_scipy_filliben_positions() {
        let normal = Normal::new(0.0, 1.0).unwrap();
        let points = normal_probability_points(&[4.0, 1.0, 3.0, 2.0], &normal);
        let expected_probabilities = [0.1591035847, 0.3854524620, 0.6145475380, 0.8408964153];

        assert_eq!(
            points.iter().map(|(_, value)| *value).collect::<Vec<_>>(),
            vec![1.0, 2.0, 3.0, 4.0]
        );
        for ((x, _), probability) in points.iter().zip(expected_probabilities) {
            assert!((normal.cdf(*x) - probability).abs() < 1e-9);
        }
    }
}
