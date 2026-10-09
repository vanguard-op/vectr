//! The embedded-library guarantees (FEAT-021, C-002).
//!
//! These exercise the crate exactly as an embedder links it: parse a scene,
//! compile it to a render model in process, export SVG and PNG in process, and
//! get a structured error for a bad document. They pin the two guarantees a
//! caller depends on — repeated calls are byte-identical, and concurrent calls
//! from many threads agree with a serial run — so a regression in either
//! surfaces here rather than inside a downstream application.

use vectr_core::{
    compile_with_style, export_svg, parse, parse_palette, DiagnosticCode, RasterOptions,
    StyleContext, SvgOptions,
};

const SCENE: &str = r##"{
  "id": "embedded-scene",
  "projectId": "embedded-project",
  "name": "Embedded scene",
  "formatVersion": "0.2",
  "canvas": { "width": 120, "height": 80, "background": "#ffffff" },
  "elements": [
    {
      "id": "badge",
      "sceneId": "embedded-scene",
      "order": 0,
      "kind": "rect",
      "geometry": { "x": 10, "y": 10, "width": 100, "height": 60, "rx": 8, "ry": 8 },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "fill": { "kind": "token", "ref": "accent" },
      "opacity": 1,
      "visible": true
    }
  ]
}"##;

const PALETTE: &str = r##"{
  "id": "embedded-palette",
  "projectId": "embedded-project",
  "name": "Embedded palette",
  "tokens": [{ "name": "accent", "value": "#3366cc" }]
}"##;

/// Compiles and exports the scene exactly as an embedder would: SVG always, and
/// PNG when the rasterizer is built in.
fn render_in_process() -> (String, Option<Vec<u8>>) {
    let scene = parse(SCENE).expect("the scene parses");
    let palette = parse_palette(PALETTE).expect("the palette parses");
    let style = StyleContext {
        palette: Some(&palette),
        ..StyleContext::default()
    };
    let model = compile_with_style(&scene, &style).expect("the scene compiles");
    let svg = export_svg(&model, &SvgOptions::default()).expect("SVG exports");

    #[cfg(feature = "rasterizer")]
    let png = Some(vectr_core::export_png(&model, &RasterOptions::default()).expect("PNG exports"));
    #[cfg(not(feature = "rasterizer"))]
    let png = None;

    (svg, png)
}

#[test]
fn a_scene_compiles_and_exports_in_process() {
    let (svg, png) = render_in_process();
    assert!(svg.contains("<svg"), "an SVG document: {svg}");
    assert!(
        svg.contains("#3366cc"),
        "the palette token resolves into the output: {svg}"
    );
    if let Some(png) = png {
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n", "a PNG image");
    }
}

#[test]
fn repeated_calls_are_byte_identical() {
    let first = render_in_process();
    let second = render_in_process();
    assert_eq!(
        first, second,
        "identical input yields identical output (NFR-010)"
    );
}

#[test]
fn an_invalid_document_is_a_structured_located_error() {
    let diagnostics = parse("{ not json").expect_err("the document is refused");
    let error = diagnostics.errors().next().expect("an error");
    assert_eq!(error.code, DiagnosticCode::PARSE);
    assert!(
        error
            .location
            .as_ref()
            .and_then(|location| location.line)
            .is_some(),
        "the error carries its location: {error}"
    );
}

#[test]
fn a_scene_that_fails_to_compile_reports_a_located_error() {
    let source = SCENE.replace(
        r#""sceneId": "embedded-scene","#,
        r#""sceneId": "embedded-scene", "parentId": "ghost","#,
    );
    let scene = parse(&source).expect("the document parses");
    let diagnostics =
        compile_with_style(&scene, &StyleContext::default()).expect_err("compilation fails");
    assert!(diagnostics.has_errors());
    let error = diagnostics.errors().next().expect("an error");
    assert!(
        error.location.is_some(),
        "a compile failure is located: {error}"
    );
}

#[test]
fn concurrent_calls_agree_with_a_serial_run() {
    let (expected_svg, expected_png) = render_in_process();
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..8).map(|_| scope.spawn(render_in_process)).collect();
        for handle in handles {
            let (svg, png) = handle.join().expect("a worker thread panicked");
            assert_eq!(svg, expected_svg, "no shared-state corruption");
            assert_eq!(png, expected_png, "no shared-state corruption");
        }
    });
}

#[cfg(not(feature = "rasterizer"))]
#[test]
fn a_build_without_a_rasterizer_reports_the_missing_capability() {
    let scene = parse(SCENE).expect("the scene parses");
    let palette = parse_palette(PALETTE).expect("the palette parses");
    let style = StyleContext {
        palette: Some(&palette),
        ..StyleContext::default()
    };
    let model = compile_with_style(&scene, &style).expect("the scene compiles");
    let diagnostics =
        vectr_core::export_png(&model, &RasterOptions::default()).expect_err("PNG is refused");
    let error = diagnostics.errors().next().expect("an error");
    assert_eq!(error.code, vectr_core::export::png::RASTERIZER);
    assert!(
        error.message.contains("rasterizer"),
        "the error names the missing capability: {}",
        error.message
    );
}
