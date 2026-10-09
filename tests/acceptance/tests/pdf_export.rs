//! Acceptance tests for PDF export (FEAT-014, C-004).
//!
//! Drives the shipped `vectr export --format pdf` command as a subprocess: the
//! document is vector, text is outlined so the page needs no installed font, the
//! page matches the requested size or defaults to the canvas with a warning, a
//! print colour profile is honoured or refused by name, and the capability is
//! gated off by default (FEAT-014, NFR-011).

mod common;

use common::*;
use serde_json::json;

/// The rollout flag that enables PDF export (FEAT-014). It is off by default.
const ENABLE: &str = "VECTR_ENABLE_PDF_EXPORT";

/// A project whose scene holds a rect and a text run, with scene metadata.
fn pdf_project(tag: &str) -> TempDir {
    let dir = TempDir::new(tag);
    dir.write("vectr.project.json", "{}");

    let box_shape = rect("box", 0, 5.0, 5.0, 30.0, 20.0);
    let wordmark = text("word", 1, 10.0, 60.0, "Hi", 20.0);
    let mut document = scene_with(vec![box_shape, wordmark], None, None);
    document["canvas"] = json!({ "width": 120.0, "height": 80.0, "background": "#ffffff" });
    document["title"] = json!("Print card");
    document["description"] = json!("A rect and a wordmark");
    write_scene_as(&dir, "card", document);
    dir
}

/// Exports the project scene to PDF with the given extra arguments and returns
/// the process output.
fn export_pdf(dir: &TempDir, out: &str, extra: &[&str]) -> std::process::Output {
    let mut args = vec!["export", "card", "--format", "pdf", "--out", out];
    args.extend_from_slice(extra);
    run_vectr_env(dir.path(), &args, &[ENABLE], &[(ENABLE, "1")])
}

#[test]
fn pdf_export_is_gated_off_by_default_and_writes_nothing() {
    let dir = pdf_project("pdf-gated");

    let output = run_vectr_env(
        dir.path(),
        &[
            "export",
            "card",
            "--format",
            "pdf",
            "--out",
            "dist/card.pdf",
        ],
        &[ENABLE],
        &[],
    );
    assert_eq!(code(&output), 2, "{}", stderr(&output));
    assert!(
        stderr(&output).contains("disabled"),
        "the gate is reported: {}",
        stderr(&output)
    );
    assert!(
        !dir.path().join("dist/card.pdf").exists(),
        "nothing is written while the capability is off"
    );
}

#[test]
fn a_pdf_export_keeps_vector_content_and_outlines_text() {
    let dir = pdf_project("pdf-vector");

    let output = export_pdf(&dir, "dist/card.pdf", &[]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let bytes = std::fs::read(dir.path().join("dist/card.pdf")).expect("the PDF was written");
    assert!(bytes.starts_with(b"%PDF-"), "a PDF document");
    let pdf = String::from_utf8_lossy(&bytes);

    assert!(
        pdf.contains(" re") && pdf.contains(" f"),
        "the vector rect is drawn with path operators: {pdf}"
    );
    assert!(
        !pdf.contains("/Subtype /Image"),
        "vector content stays vector, not rasterized: {pdf}"
    );
    assert!(
        !pdf.contains("/Font") && !pdf.contains("/FontFile"),
        "text is outlined, so the PDF depends on no installed font: {pdf}"
    );
}

#[test]
fn an_explicit_page_size_matches_the_requested_size() {
    let dir = pdf_project("pdf-page-size");

    let output = export_pdf(
        &dir,
        "dist/card.pdf",
        &["--width", "612", "--height", "792"],
    );
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let bytes = std::fs::read(dir.path().join("dist/card.pdf")).expect("pdf");
    let pdf = String::from_utf8_lossy(&bytes);
    assert!(
        pdf.contains("/MediaBox [0 0 612 792]"),
        "the page matches the requested size: {pdf}"
    );
    assert!(
        !stderr(&output).contains("W_PDF_NO_PAGE_SIZE"),
        "a requested page size is not warned about: {}",
        stderr(&output)
    );
}

#[test]
fn a_pdf_without_a_page_size_defaults_to_the_canvas_and_warns() {
    let dir = pdf_project("pdf-default-size");

    let output = export_pdf(&dir, "dist/card.pdf", &[]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let bytes = std::fs::read(dir.path().join("dist/card.pdf")).expect("pdf");
    let pdf = String::from_utf8_lossy(&bytes);
    assert!(
        pdf.contains("/MediaBox [0 0 120 80]"),
        "the page defaults to the canvas size: {pdf}"
    );
    assert!(
        stderr(&output).contains("W_PDF_NO_PAGE_SIZE"),
        "the default is warned about: {}",
        stderr(&output)
    );
}

#[test]
fn a_print_colour_profile_is_honoured_and_an_unknown_one_is_refused() {
    let dir = pdf_project("pdf-profile");

    let cmyk = export_pdf(&dir, "dist/cmyk.pdf", &["--profile", "cmyk"]);
    assert_eq!(code(&cmyk), 0, "{}", stderr(&cmyk));
    let bytes = std::fs::read(dir.path().join("dist/cmyk.pdf")).expect("pdf");
    let pdf = String::from_utf8_lossy(&bytes);
    assert!(
        pdf.contains(" k\n") || pdf.contains(" K\n"),
        "the cmyk profile selects a cmyk colour space: {pdf}"
    );
    assert!(
        !pdf.contains(" rg\n"),
        "the cmyk profile does not emit rgb fills: {pdf}"
    );

    let unknown = export_pdf(&dir, "dist/bad.pdf", &["--profile", "adobe-rgb"]);
    assert_ne!(code(&unknown), 0, "{}", stderr(&unknown));
    assert!(
        stderr(&unknown).contains("adobe-rgb"),
        "the unknown profile is named: {}",
        stderr(&unknown)
    );
    assert!(
        !dir.path().join("dist/bad.pdf").exists(),
        "nothing is written for a refused profile"
    );
}

/// A print profile that cannot carry per-paint transparency flattens it and
/// reports that it did, while a profile that can carry it reports nothing
/// (FEAT-014).
#[test]
fn a_profile_that_cannot_carry_transparency_flattens_it_and_reports_it() {
    let dir = TempDir::new("pdf-transparency");
    dir.write("vectr.project.json", "{}");
    dir.write(
        "palettes/brand.json",
        &palette("brand", &[("fade", "#ff000080")]),
    );
    let mut card = rect("card", 0, 5.0, 5.0, 30.0, 20.0);
    card["fill"] = token_paint("fade");
    let mut document = scene_with(vec![card], None, Some("brand"));
    document["canvas"] = json!({ "width": 120.0, "height": 80.0, "background": "#ffffff" });
    write_scene_as(&dir, "card", document);

    let cmyk = export_pdf(&dir, "dist/cmyk.pdf", &["--profile", "cmyk"]);
    assert_eq!(code(&cmyk), 0, "{}", stderr(&cmyk));
    assert!(
        stderr(&cmyk).contains("W_PDF_TRANSPARENCY_FLATTENED"),
        "the flattened transparency is reported: {}",
        stderr(&cmyk)
    );

    let srgb = export_pdf(&dir, "dist/srgb.pdf", &["--profile", "srgb"]);
    assert_eq!(code(&srgb), 0, "{}", stderr(&srgb));
    assert!(
        !stderr(&srgb).contains("W_PDF_TRANSPARENCY_FLATTENED"),
        "rgb carries transparency, so nothing is flattened: {}",
        stderr(&srgb)
    );
}
