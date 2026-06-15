use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::chart::ChartImage;

const SLIDE_WIDTH: i64 = 12_192_000;
const SLIDE_HEIGHT: i64 = 6_858_000;
const IMAGE_WIDTH: i64 = 2_560_000;
const IMAGE_HEIGHT: i64 = 1_600_000;
const IMAGE_GAP_X: i64 = 120_000;
const IMAGE_GAP_Y: i64 = 120_000;

struct ReportSlide<'a> {
    title: &'static str,
    images: Vec<&'a ChartImage>,
}

pub(super) fn create_report(
    template_path: &Path,
    output_path: &Path,
    chart_images: &[ChartImage],
) -> Result<()> {
    if chart_images.is_empty() {
        bail!("No chart images were generated for the PowerPoint report");
    }

    let slides = report_slides(chart_images);
    let template_file = File::open(template_path)
        .with_context(|| format!("Unable to open template {}", template_path.display()))?;
    let mut archive = ZipArchive::new(template_file)
        .with_context(|| format!("Invalid PowerPoint template {}", template_path.display()))?;

    let output_file = File::create(output_path)
        .with_context(|| format!("Unable to create {}", output_path.display()))?;
    let mut writer = ZipWriter::new(output_file);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let name = entry.name().to_owned();
        if matches!(
            name.as_str(),
            "ppt/presentation.xml"
                | "ppt/_rels/presentation.xml.rels"
                | "[Content_Types].xml"
                | "docProps/app.xml"
        ) {
            continue;
        }
        if entry.is_dir() {
            writer.add_directory(name, options)?;
            continue;
        }
        writer.start_file(name, options)?;
        std::io::copy(&mut entry, &mut writer)?;
    }

    let presentation = read_zip_text(&mut archive, "ppt/presentation.xml")?;
    write_text_entry(
        &mut writer,
        "ppt/presentation.xml",
        &add_slides_to_presentation(&presentation, slides.len())?,
        options,
    )?;

    let relationships = read_zip_text(&mut archive, "ppt/_rels/presentation.xml.rels")?;
    write_text_entry(
        &mut writer,
        "ppt/_rels/presentation.xml.rels",
        &add_slide_relationships(&relationships, slides.len())?,
        options,
    )?;

    let content_types = read_zip_text(&mut archive, "[Content_Types].xml")?;
    write_text_entry(
        &mut writer,
        "[Content_Types].xml",
        &add_slide_content_types(&content_types, slides.len())?,
        options,
    )?;

    let app_properties = read_zip_text(&mut archive, "docProps/app.xml")?;
    write_text_entry(
        &mut writer,
        "docProps/app.xml",
        &update_slide_count(&app_properties, slides.len()),
        options,
    )?;

    let mut media_index = 5usize;
    for (slide_index, slide) in slides.iter().enumerate() {
        let slide_number = slide_index + 1;
        let mut image_relationships = Vec::new();
        for (image_index, chart) in slide.images.iter().enumerate() {
            media_index += 1;
            let media_name = format!("ppt/media/chartify_image_{media_index}.png");
            writer.start_file(&media_name, options)?;
            let mut file = File::open(&chart.path)
                .with_context(|| format!("Unable to open {}", chart.path.display()))?;
            std::io::copy(&mut file, &mut writer)?;
            image_relationships.push((
                image_index + 2,
                format!("../media/chartify_image_{media_index}.png"),
            ));
        }

        write_text_entry(
            &mut writer,
            &format!("ppt/slides/slide{slide_number}.xml"),
            &slide_xml(slide),
            options,
        )?;
        write_text_entry(
            &mut writer,
            &format!("ppt/slides/_rels/slide{slide_number}.xml.rels"),
            &slide_relationships_xml(&image_relationships),
            options,
        )?;
    }

    writer.finish()?;
    Ok(())
}

fn report_slides(chart_images: &[ChartImage]) -> Vec<ReportSlide<'_>> {
    let significant = chart_images
        .iter()
        .filter(|image| image.significant)
        .collect::<Vec<_>>();
    let comparable = chart_images
        .iter()
        .filter(|image| !image.significant)
        .collect::<Vec<_>>();
    let mut slides = Vec::new();

    for images in significant.chunks(8) {
        slides.push(ReportSlide {
            title: "Metrics Mismatch",
            images: images.to_vec(),
        });
    }
    for images in comparable.chunks(8) {
        slides.push(ReportSlide {
            title: "Metrics Comparable",
            images: images.to_vec(),
        });
    }
    slides
}

fn slide_xml(slide: &ReportSlide<'_>) -> String {
    let grid_width = IMAGE_WIDTH * 4 + IMAGE_GAP_X * 3;
    let grid_height = IMAGE_HEIGHT * 2 + IMAGE_GAP_Y;
    let margin_x = (SLIDE_WIDTH - grid_width) / 2;
    let margin_y = (SLIDE_HEIGHT - grid_height) / 2;
    let mut pictures = String::new();

    for (index, chart) in slide.images.iter().enumerate() {
        let x = margin_x + (index % 4) as i64 * (IMAGE_WIDTH + IMAGE_GAP_X);
        let y = margin_y + (index / 4) as i64 * (IMAGE_HEIGHT + IMAGE_GAP_Y);
        let relationship_id = index + 2;
        let shape_id = index + 3;
        pictures.push_str(&format!(
            r#"<p:pic>
<p:nvPicPr>
<p:cNvPr id="{shape_id}" name="Chart {shape_id}" descr="{description}"/>
<p:cNvPicPr><a:picLocks noChangeAspect="1"/></p:cNvPicPr>
<p:nvPr/>
</p:nvPicPr>
<p:blipFill>
<a:blip r:embed="rId{relationship_id}"/>
<a:stretch><a:fillRect/></a:stretch>
</p:blipFill>
<p:spPr>
<a:xfrm><a:off x="{x}" y="{y}"/><a:ext cx="{IMAGE_WIDTH}" cy="{IMAGE_HEIGHT}"/></a:xfrm>
<a:prstGeom prst="rect"><a:avLst/></a:prstGeom>
</p:spPr>
</p:pic>"#,
            description = escape_xml(&chart.item),
        ));
    }

    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
<p:cSld>
<p:spTree>
<p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
<p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr>
<p:sp>
<p:nvSpPr><p:cNvPr id="2" name="Title 1"/><p:cNvSpPr><a:spLocks noGrp="1"/></p:cNvSpPr><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr>
<p:spPr/>
<p:txBody>
<a:bodyPr anchor="b" anchorCtr="0"><a:noAutofit/></a:bodyPr>
<a:lstStyle/>
<a:p><a:r><a:rPr lang="en-US"/><a:t>{title}</a:t></a:r><a:endParaRPr lang="en-US"/></a:p>
</p:txBody>
</p:sp>
{pictures}
</p:spTree>
</p:cSld>
<p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr>
</p:sld>"#,
        title = escape_xml(slide.title),
    )
}

fn slide_relationships_xml(image_relationships: &[(usize, String)]) -> String {
    let mut relationships = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout3.xml"/>"#,
    );
    for (id, target) in image_relationships {
        relationships.push_str(&format!(
            r#"<Relationship Id="rId{id}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="{target}"/>"#
        ));
    }
    relationships.push_str("</Relationships>");
    relationships
}

fn add_slides_to_presentation(xml: &str, slide_count: usize) -> Result<String> {
    let marker = "<p:sldSz";
    let position = xml
        .find(marker)
        .context("Template presentation.xml has no slide size")?;
    let slide_ids = (0..slide_count)
        .map(|index| format!(r#"<p:sldId id="{}" r:id="rId{}"/>"#, 256 + index, 9 + index))
        .collect::<String>();
    let slide_list = format!("<p:sldIdLst>{slide_ids}</p:sldIdLst>");
    let mut result = xml.to_owned();
    result.insert_str(position, &slide_list);
    Ok(result)
}

fn add_slide_relationships(xml: &str, slide_count: usize) -> Result<String> {
    let marker = "</Relationships>";
    let position = xml
        .rfind(marker)
        .context("Template presentation relationships are invalid")?;
    let additions = (0..slide_count)
        .map(|index| {
            format!(
                r#"<Relationship Id="rId{}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide{}.xml"/>"#,
                9 + index,
                index + 1
            )
        })
        .collect::<String>();
    let mut result = xml.to_owned();
    result.insert_str(position, &additions);
    Ok(result)
}

fn add_slide_content_types(xml: &str, slide_count: usize) -> Result<String> {
    let marker = "</Types>";
    let position = xml
        .rfind(marker)
        .context("Template content types are invalid")?;
    let additions = (0..slide_count)
        .map(|index| {
            format!(
                r#"<Override PartName="/ppt/slides/slide{}.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>"#,
                index + 1
            )
        })
        .collect::<String>();
    let mut result = xml.to_owned();
    result.insert_str(position, &additions);
    Ok(result)
}

fn update_slide_count(xml: &str, slide_count: usize) -> String {
    xml.replace(
        "<Slides>0</Slides>",
        &format!("<Slides>{slide_count}</Slides>"),
    )
}

fn read_zip_text(archive: &mut ZipArchive<File>, name: &str) -> Result<String> {
    let mut entry = archive
        .by_name(name)
        .with_context(|| format!("Template is missing {name}"))?;
    let mut text = String::new();
    entry.read_to_string(&mut text)?;
    Ok(text)
}

fn write_text_entry(
    writer: &mut ZipWriter<File>,
    name: &str,
    content: &str,
    options: SimpleFileOptions,
) -> Result<()> {
    writer.start_file(name, options)?;
    writer.write_all(content.as_bytes())?;
    Ok(())
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_significance_sections_and_pages() {
        let images = (0..10)
            .map(|index| ChartImage {
                item: format!("M{index}"),
                path: format!("M{index}.png").into(),
                significant: index < 2,
            })
            .collect::<Vec<_>>();
        let slides = report_slides(&images);
        assert_eq!(slides.len(), 2);
        assert_eq!(slides[0].title, "Metrics Mismatch");
        assert_eq!(slides[0].images.len(), 2);
        assert_eq!(slides[1].title, "Metrics Comparable");
        assert_eq!(slides[1].images.len(), 8);
    }
}
