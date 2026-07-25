//! Synthetic tiny 3MF ZIP tests (no copyrighted meshes).
//!
//! Naming: `function_or_feature__condition__expected_result`

#![allow(non_snake_case)] // track test naming: feature__condition__expected

use std::io::{Cursor, Write};

use camino::Utf8Path;
use serde_json::json;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::analyze::analyze_archive;
use crate::convert::{
    ArchiveConvertOptions, ConvertStrategy, convert_archives, format_report_markdown,
    refuse_output_equals_input,
};
use crate::model_settings::parse_model_settings;
use crate::opc::{CONTENT_TYPES, PACKAGE_RELS, normalize_opc_part_name};
use crate::paths::{
    CUSTOM_GCODE_PER_LAYER, FILAMENT_SEQUENCE, MODEL_SETTINGS, PROJECT_SETTINGS, ROOT_MODEL,
    SLICE_INFO, normalize_zip_path,
};
use crate::settings::{
    BED_COMPARE_EPS_MM, bed_compare_message, bed_size_mm, bed_source_exceeds_template,
    parse_project_settings, string_array_field, string_field,
};
use crate::slot_map::SlotMap;
use crate::zip_util::read_member_bytes;

const MODEL_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<model unit="millimeter">
 <metadata name="Application">BambuStudio-test</metadata>
 <resources>
  <object id="1" type="model">
   <mesh>
    <vertices>
     <vertex x="0" y="0" z="0"/>
     <vertex x="1" y="0" z="0"/>
     <vertex x="0" y="1" z="0"/>
    </vertices>
    <triangles>
     <triangle v1="0" v2="1" v3="2"/>
    </triangles>
   </mesh>
  </object>
 </resources>
 <build>
  <item objectid="1"/>
 </build>
</model>
"#;

const NESTED_OBJECT: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<model unit="millimeter">
 <resources>
  <object id="1" type="model">
   <mesh>
    <vertices>
     <vertex x="0" y="0" z="0"/>
     <vertex x="1" y="0" z="0"/>
     <vertex x="0" y="1" z="0"/>
    </vertices>
    <triangles>
     <triangle v1="0" v2="1" v3="2"/>
    </triangles>
   </mesh>
  </object>
 </resources>
</model>
"#;

const MODEL_SETTINGS_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<config>
  <object id="1">
    <metadata key="name" value="PartA"/>
    <metadata key="extruder" value="1"/>
    <part id="1" subtype="normal_part">
      <metadata key="extruder" value="1"/>
    </part>
    <part id="2" subtype="normal_part">
      <metadata key="extruder" value="2"/>
    </part>
    <part id="3" subtype="normal_part">
      <metadata key="extruder" value="3"/>
    </part>
  </object>
  <plate>
    <metadata key="plater_id" value="1"/>
  </plate>
  <plate>
    <metadata key="plater_id" value="2"/>
  </plate>
</config>
"#;

fn project_settings_json(
    printer: &str,
    colours: &[&str],
    types: &[&str],
    multi: Option<&[&str]>,
) -> Vec<u8> {
    project_settings_json_ex(printer, colours, types, multi, None, None)
}

fn project_settings_json_ex(
    printer: &str,
    colours: &[&str],
    types: &[&str],
    multi: Option<&[&str]>,
    printable_area: Option<&[&str]>,
    machine_start_gcode: Option<&str>,
) -> Vec<u8> {
    let area = printable_area.unwrap_or(&["0x0", "300x0", "300x270", "0x270"]);
    let mut obj = json!({
        "printer_model": printer,
        "filament_colour": colours,
        "filament_type": types,
        "filament_settings_id": ["tpl-A", "tpl-B", "tpl-C", "tpl-D"],
        "printable_area": area,
    });
    if let Some(m) = multi {
        obj.as_object_mut()
            .expect("object")
            .insert("filament_multi_colour".into(), json!(m));
    }
    if let Some(gcode) = machine_start_gcode {
        obj.as_object_mut()
            .expect("object")
            .insert("machine_start_gcode".into(), json!(gcode));
    }
    serde_json::to_vec_pretty(&obj).expect("serialize")
}

fn options() -> SimpleFileOptions {
    SimpleFileOptions::default().compression_method(CompressionMethod::Deflated)
}

fn build_source_zip() -> Vec<u8> {
    let buf = Cursor::new(Vec::new());
    let mut z = ZipWriter::new(buf);
    let opt = options();

    z.start_file(ROOT_MODEL, opt).unwrap();
    z.write_all(MODEL_XML.as_bytes()).unwrap();

    z.start_file("3D/Objects/object_1.model", opt).unwrap();
    z.write_all(NESTED_OBJECT.as_bytes()).unwrap();

    z.start_file(MODEL_SETTINGS, opt).unwrap();
    z.write_all(MODEL_SETTINGS_XML.as_bytes()).unwrap();

    z.start_file(PROJECT_SETTINGS, opt).unwrap();
    z.write_all(&project_settings_json(
        "Bambu Lab H2C",
        &["#AABBCC", "#DDEEFF", "#112233", "#445566"],
        &["PLA", "PLA", "PLA", "PLA"],
        Some(&["#AABBCC", "#DDEEFF", "#112233", "#445566"]),
    ))
    .unwrap();

    z.start_file(CUSTOM_GCODE_PER_LAYER, opt).unwrap();
    z.write_all(b"<custom_gcodes_per_layer/>").unwrap();

    z.start_file(FILAMENT_SEQUENCE, opt).unwrap();
    z.write_all(b"{}").unwrap();

    z.start_file("Metadata/layer_heights_profile.txt", opt)
        .unwrap();
    z.write_all(b"0\n").unwrap();

    z.start_file("Metadata/plate_1.gcode", opt).unwrap();
    z.write_all(b"; fake gcode\n").unwrap();

    z.start_file(SLICE_INFO, opt).unwrap();
    z.write_all(b"<config><header/></config>").unwrap();

    z.start_file("[Content_Types].xml", opt).unwrap();
    z.write_all(b"<?xml version=\"1.0\"?><Types/>").unwrap();

    let cursor = z.finish().unwrap();
    cursor.into_inner()
}

fn build_template_zip() -> Vec<u8> {
    let buf = Cursor::new(Vec::new());
    let mut z = ZipWriter::new(buf);
    let opt = options();

    z.start_file(ROOT_MODEL, opt).unwrap();
    z.write_all(
        br#"<?xml version="1.0"?><model><metadata name="Application">BambuStudio-2.3.1</metadata></model>"#,
    )
    .unwrap();

    z.start_file(PROJECT_SETTINGS, opt).unwrap();
    // Template colours deliberately differ from source
    z.write_all(&project_settings_json(
        "WonderMaker ZR Ultra S",
        &["#FFFFFF", "#FFFF00", "#FF0000", "#0000FF"],
        &["PETG", "PETG", "PETG", "PETG"],
        None,
    ))
    .unwrap();

    z.start_file(MODEL_SETTINGS, opt).unwrap();
    z.write_all(br#"<?xml version="1.0"?><config><plate/></config>"#)
        .unwrap();

    let cursor = z.finish().unwrap();
    cursor.into_inner()
}

fn open_mem(bytes: &[u8]) -> ZipArchive<Cursor<Vec<u8>>> {
    ZipArchive::new(Cursor::new(bytes.to_vec())).expect("open zip")
}

#[test]
fn analyze_synthetic__reports_printer_plates_filaments_paint_flag() {
    let bytes = build_source_zip();
    let mut archive = open_mem(&bytes);
    let a = analyze_archive(&mut archive, "synth-source.3mf").expect("analyze");

    assert_eq!(a.printer_model.as_deref(), Some("Bambu Lab H2C"));
    assert_eq!(a.application.as_deref(), Some("BambuStudio-test"));
    assert_eq!(a.plate_count, 2);
    assert_eq!(a.filaments.len(), 4);
    assert_eq!(a.filaments[0].colour, "#AABBCC");
    assert_eq!(a.filaments[0].type_, "PLA");
    assert!(!a.has_paint_color);
    assert_eq!(a.paint_color_count, 0);
    assert_eq!(a.extruder_histogram.get(&1), Some(&2)); // object + part
    assert_eq!(a.extruder_histogram.get(&2), Some(&1));
    assert_eq!(a.extruder_histogram.get(&3), Some(&1));
    assert_eq!(a.used_source_slots, vec![1, 2, 3]);
    assert!(a.has_gcode);
}

#[test]
fn convert_graft__output_project_settings__has_template_printer_model() {
    let source = build_source_zip();
    let template = build_template_zip();
    let mut src = open_mem(&source);
    let mut tpl = open_mem(&template);
    let out_buf = Cursor::new(Vec::new());
    let report = convert_archives(
        &mut src,
        &mut tpl,
        out_buf,
        &ArchiveConvertOptions::default(),
    )
    .expect("convert");
    assert_eq!(
        report.output_printer.as_deref(),
        Some("WonderMaker ZR Ultra S")
    );
    assert_eq!(report.source_printer.as_deref(), Some("Bambu Lab H2C"));

    // Re-run to inspect ZIP (convert_archives consumes writer; rebuild)
    let mut src = open_mem(&source);
    let mut tpl = open_mem(&template);
    let mut out_buf = Cursor::new(Vec::new());
    convert_archives(
        &mut src,
        &mut tpl,
        &mut out_buf,
        &ArchiveConvertOptions::default(),
    )
    .expect("convert2");
    let out_bytes = out_buf.into_inner();
    let mut out_zip = open_mem(&out_bytes);
    let ps = read_member_bytes(&mut out_zip, PROJECT_SETTINGS).expect("ps");
    let settings = parse_project_settings(&ps).expect("parse");
    assert_eq!(
        string_field(&settings, "printer_model").as_deref(),
        Some("WonderMaker ZR Ultra S")
    );

    // Root model Application must be stamped from template so Orca does not warn
    // "The 3MF was created by BambuStudio" for a foreign Studio version.
    let root = read_member_bytes(&mut out_zip, ROOT_MODEL).expect("root model");
    let app = crate::model_meta::read_application_metadata(&root);
    assert_eq!(
        app.as_deref(),
        Some("BambuStudio-2.3.1"),
        "Application should match Wonderprint template, got {app:?}"
    );
}

#[test]
fn colour_patch__opt_in_copy_source_colours__output_has_source_filament_colour() {
    let source = build_source_zip();
    let template = build_template_zip();
    let mut src = open_mem(&source);
    let mut tpl = open_mem(&template);
    let mut out_buf = Cursor::new(Vec::new());
    convert_archives(
        &mut src,
        &mut tpl,
        &mut out_buf,
        &ArchiveConvertOptions::default().with_copy_source_colours(true),
    )
    .expect("convert");
    let out_bytes = out_buf.into_inner();
    let mut out_zip = open_mem(&out_bytes);
    let ps = read_member_bytes(&mut out_zip, PROJECT_SETTINGS).expect("ps");
    let settings = parse_project_settings(&ps).expect("parse");

    let colours = string_array_field(&settings, "filament_colour");
    assert_eq!(
        colours,
        vec!["#AABBCC", "#DDEEFF", "#112233", "#445566"],
        "with copy_source_colours, output must match source palette"
    );
    let multi = string_array_field(&settings, "filament_multi_colour");
    assert_eq!(multi, vec!["#AABBCC", "#DDEEFF", "#112233", "#445566"]);
    // Template filament_settings_id preserved
    assert_eq!(
        string_array_field(&settings, "filament_settings_id"),
        vec!["tpl-A", "tpl-B", "tpl-C", "tpl-D"]
    );
}

#[test]
fn colour_default__keep_template_filament_colours() {
    let source = build_source_zip();
    let template = build_template_zip();
    let mut src = open_mem(&source);
    let mut tpl = open_mem(&template);
    let mut out_buf = Cursor::new(Vec::new());
    convert_archives(
        &mut src,
        &mut tpl,
        &mut out_buf,
        &ArchiveConvertOptions::default(), // copy_source_colours = false
    )
    .expect("convert");
    let mut out_zip = open_mem(&out_buf.into_inner());
    let ps = read_member_bytes(&mut out_zip, PROJECT_SETTINGS).expect("ps");
    let settings = parse_project_settings(&ps).expect("parse");
    // Template palette white/yellow/red/blue — not source AABBCC…
    assert_eq!(
        string_array_field(&settings, "filament_colour"),
        vec!["#FFFFFF", "#FFFF00", "#FF0000", "#0000FF"]
    );
}

#[test]
fn strip_list__removes_custom_gcode_and_gcode_members() {
    let source = build_source_zip();
    let template = build_template_zip();
    let mut src = open_mem(&source);
    let mut tpl = open_mem(&template);
    let mut out_buf = Cursor::new(Vec::new());
    let report = convert_archives(
        &mut src,
        &mut tpl,
        &mut out_buf,
        &ArchiveConvertOptions::default(),
    )
    .expect("convert");

    assert!(
        report
            .stripped_members
            .iter()
            .any(|m| m == CUSTOM_GCODE_PER_LAYER)
    );
    assert!(
        report
            .stripped_members
            .iter()
            .any(|m| m.ends_with(".gcode"))
    );

    let out_bytes = out_buf.into_inner();
    let mut out_zip = open_mem(&out_bytes);
    let names: Vec<String> = (0..out_zip.len())
        .map(|i| normalize_zip_path(out_zip.by_index(i).unwrap().name()))
        .collect();

    assert!(!names.iter().any(|n| n == CUSTOM_GCODE_PER_LAYER));
    assert!(!names.iter().any(|n| n.ends_with(".gcode")));
    assert!(!names.iter().any(|n| n == FILAMENT_SEQUENCE));
    assert!(
        !names
            .iter()
            .any(|n| n == "Metadata/layer_heights_profile.txt")
    );
}

#[test]
fn zip_member_names__use_only_forward_slash() {
    let source = build_source_zip();
    let template = build_template_zip();
    let mut src = open_mem(&source);
    let mut tpl = open_mem(&template);
    let mut out_buf = Cursor::new(Vec::new());
    convert_archives(
        &mut src,
        &mut tpl,
        &mut out_buf,
        &ArchiveConvertOptions::default(),
    )
    .expect("convert");
    let out_bytes = out_buf.into_inner();
    let mut out_zip = open_mem(&out_bytes);
    for i in 0..out_zip.len() {
        let name = out_zip.by_index(i).unwrap().name().to_string();
        assert!(
            !name.contains('\\'),
            "ZIP member must not contain backslash: {name}"
        );
        assert_eq!(name, normalize_zip_path(&name));
    }
}

#[test]
fn convert_refuses__output_equals_input() {
    let p = Utf8Path::new(r"C:\dev\Wondermaker\output\same.3mf");
    let err = refuse_output_equals_input(p, p).expect_err("must refuse");
    let msg = err.to_string();
    assert!(
        msg.contains("refused") || msg.contains("must differ") || msg.contains("overwrite"),
        "unexpected error: {msg}"
    );
}

#[test]
fn geometry_preserved__nested_objects_and_model_settings_after_convert() {
    let source = build_source_zip();
    let template = build_template_zip();
    let mut src = open_mem(&source);
    let mut tpl = open_mem(&template);
    let mut out_buf = Cursor::new(Vec::new());
    convert_archives(
        &mut src,
        &mut tpl,
        &mut out_buf,
        &ArchiveConvertOptions::default(),
    )
    .expect("convert");
    let out_bytes = out_buf.into_inner();
    let mut out_zip = open_mem(&out_bytes);

    // Nested object model present
    let nested = read_member_bytes(&mut out_zip, "3D/Objects/object_1.model").expect("nested");
    assert_eq!(nested, NESTED_OBJECT.as_bytes());

    // model_settings raw-copied (extruders preserved)
    let ms = read_member_bytes(&mut out_zip, MODEL_SETTINGS).expect("model_settings");
    let text = String::from_utf8(ms).expect("utf8");
    assert!(text.contains("extruder"));
    assert!(text.contains("value=\"2\""));
    assert!(text.contains("<plate>"));

    // Root model present
    let root = read_member_bytes(&mut out_zip, ROOT_MODEL).expect("root");
    assert!(!root.is_empty());
}

/// Disk convert refuse path using tempfile (covers convert() entry).
#[test]
fn convert_disk__output_equals_input__error() {
    use crate::convert::{ConvertOptions, convert};

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("same.3mf");
    std::fs::write(&path, build_source_zip()).expect("write");
    let utf = camino::Utf8PathBuf::from_path_buf(path).expect("utf8");
    // Use same path as source and output; template can be anything that won't be opened
    // if equality check runs first.
    let opts = ConvertOptions::new(&utf, &utf, &utf);
    let err = convert(&opts).expect_err("must fail");
    assert!(
        matches!(err, crate::error::Error::OutputEqualsInput(_)),
        "got {err:?}"
    );
}

/// Ensure raw bytes of model_settings are identical after convert (raw_copy).
#[test]
fn model_settings__byte_identical_after_convert() {
    let source = build_source_zip();
    let template = build_template_zip();
    let mut src = open_mem(&source);
    let before = read_member_bytes(&mut src, MODEL_SETTINGS).expect("before");
    let mut src = open_mem(&source);
    let mut tpl = open_mem(&template);
    let mut out_buf = Cursor::new(Vec::new());
    convert_archives(
        &mut src,
        &mut tpl,
        &mut out_buf,
        &ArchiveConvertOptions::default(),
    )
    .expect("convert");
    let mut out_zip = open_mem(&out_buf.into_inner());
    let after = read_member_bytes(&mut out_zip, MODEL_SETTINGS).expect("after");
    assert_eq!(before, after);
}

/// Source ZIP with backslash member names must emit only `/` paths after convert.
#[test]
fn convert__backslash_member_names__output_uses_forward_slash_only() {
    let geometry = NESTED_OBJECT.as_bytes();
    let source = {
        let buf = Cursor::new(Vec::new());
        let mut z = ZipWriter::new(buf);
        let opt = options();
        // Deliberately non-canonical OPC names (Windows-style separators / leading slash).
        z.start_file(r"3D\Objects\object_bs.model", opt).unwrap();
        z.write_all(geometry).unwrap();
        z.start_file("/Metadata/project_settings.config", opt)
            .unwrap();
        z.write_all(&project_settings_json(
            "Bambu Lab H2C",
            &["#AABBCC"],
            &["PLA"],
            None,
        ))
        .unwrap();
        z.start_file(r"Metadata\model_settings.config", opt)
            .unwrap();
        z.write_all(MODEL_SETTINGS_XML.as_bytes()).unwrap();
        z.finish().unwrap().into_inner()
    };
    let template = build_template_zip();
    let mut src = open_mem(&source);
    let mut tpl = open_mem(&template);
    let mut out_buf = Cursor::new(Vec::new());
    convert_archives(
        &mut src,
        &mut tpl,
        &mut out_buf,
        &ArchiveConvertOptions::default(),
    )
    .expect("convert");
    let out_bytes = out_buf.into_inner();
    let mut out_zip = open_mem(&out_bytes);

    for i in 0..out_zip.len() {
        let name = out_zip.by_index(i).unwrap().name().to_string();
        assert!(
            !name.contains('\\'),
            "output member must not contain backslash: {name}"
        );
        assert!(
            !name.starts_with('/'),
            "output member must not have leading slash: {name}"
        );
        assert_eq!(
            name,
            normalize_zip_path(&name),
            "output member must already be canonical: {name}"
        );
    }

    // Geometry content preserved under the normalized path.
    let nested = read_member_bytes(&mut out_zip, "3D/Objects/object_bs.model").expect("geometry");
    assert_eq!(nested, geometry);
    let ms = read_member_bytes(&mut out_zip, MODEL_SETTINGS).expect("model_settings");
    assert_eq!(ms, MODEL_SETTINGS_XML.as_bytes());
}

/// Corrupt project_settings JSON must fail analyze (not look like missing metadata).
#[test]
fn analyze__corrupt_project_settings_json__returns_error() {
    let bytes = {
        let buf = Cursor::new(Vec::new());
        let mut z = ZipWriter::new(buf);
        let opt = options();
        z.start_file(PROJECT_SETTINGS, opt).unwrap();
        z.write_all(b"{not valid json!!!").unwrap();
        z.finish().unwrap().into_inner()
    };
    let mut archive = open_mem(&bytes);
    let err = analyze_archive(&mut archive, "corrupt-ps.3mf").expect_err("must fail");
    assert!(
        matches!(err, crate::error::Error::Json { .. }),
        "expected JSON error, got {err:?}"
    );
}

/// Corrupt/unparseable model_settings XML must fail analyze.
#[test]
fn analyze__corrupt_model_settings_xml__returns_error() {
    let bytes = {
        let buf = Cursor::new(Vec::new());
        let mut z = ZipWriter::new(buf);
        let opt = options();
        z.start_file(PROJECT_SETTINGS, opt).unwrap();
        z.write_all(&project_settings_json(
            "Bambu Lab H2C",
            &["#AABBCC"],
            &["PLA"],
            None,
        ))
        .unwrap();
        z.start_file(MODEL_SETTINGS, opt).unwrap();
        // Unclosed tag / garbage that quick-xml cannot finish parsing.
        z.write_all(b"<config><plate><metadata key=\"extruder\" value=\"1\"")
            .unwrap();
        z.finish().unwrap().into_inner()
    };
    let mut archive = open_mem(&bytes);
    let err = analyze_archive(&mut archive, "corrupt-ms.3mf").expect_err("must fail");
    assert!(
        matches!(err, crate::error::Error::Xml { .. }),
        "expected XML error, got {err:?}"
    );
}

/// Missing optional members (root model, model_settings) still analyzes successfully.
#[test]
fn analyze__missing_optional_members__still_ok() {
    let bytes = {
        let buf = Cursor::new(Vec::new());
        let mut z = ZipWriter::new(buf);
        let opt = options();
        z.start_file(PROJECT_SETTINGS, opt).unwrap();
        z.write_all(&project_settings_json(
            "Bambu Lab H2C",
            &["#AABBCC", "#DDEEFF"],
            &["PLA", "PLA"],
            None,
        ))
        .unwrap();
        // No ROOT_MODEL, no MODEL_SETTINGS.
        z.start_file("[Content_Types].xml", opt).unwrap();
        z.write_all(b"<?xml version=\"1.0\"?><Types/>").unwrap();
        z.finish().unwrap().into_inner()
    };
    let mut archive = open_mem(&bytes);
    let a = analyze_archive(&mut archive, "minimal.3mf").expect("analyze");
    assert_eq!(a.printer_model.as_deref(), Some("Bambu Lab H2C"));
    assert!(a.application.is_none());
    assert_eq!(a.plate_count, 0);
    assert!(a.extruder_histogram.is_empty());
    assert_eq!(a.filaments.len(), 2);
    // No extruder/paint → default used slot [1] (UI must still map something).
    assert_eq!(a.used_source_slots, vec![1]);
}

/// Paint-decoded slots are unioned into used_source_slots with extruder histogram.
#[test]
fn analyze__painted_model__used_source_slots_includes_paint() {
    let bytes = build_source_zip_painted();
    let mut archive = open_mem(&bytes);
    let a = analyze_archive(&mut archive, "painted.3mf").expect("analyze");
    assert!(a.has_paint_color);
    // Histogram keys 1,2,3; paint "4"→slot1, "DC"→slot16
    assert!(a.used_source_slots.contains(&1));
    assert!(a.used_source_slots.contains(&2));
    assert!(a.used_source_slots.contains(&3));
    assert!(
        a.used_source_slots.contains(&16),
        "paint code DC (slot 16) must be in used_source_slots: {:?}",
        a.used_source_slots
    );
    // Sorted unique
    let mut sorted = a.used_source_slots.clone();
    sorted.sort_unstable();
    assert_eq!(a.used_source_slots, sorted);
}

// --- 0002: paint / remap / report integration ---

const PAINTED_MODEL: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<model unit="millimeter">
 <resources>
  <object id="1" type="model">
   <mesh>
    <vertices>
     <vertex x="0" y="0" z="0"/>
     <vertex x="1" y="0" z="0"/>
     <vertex x="0" y="1" z="0"/>
    </vertices>
    <triangles>
     <triangle v1="0" v2="1" v3="2" paint_color="4"/>
     <triangle v1="0" v2="1" v3="2" paint_color="DC"/>
    </triangles>
   </mesh>
  </object>
 </resources>
</model>
"#;

fn build_source_zip_painted() -> Vec<u8> {
    let buf = Cursor::new(Vec::new());
    let mut z = ZipWriter::new(buf);
    let opt = options();

    z.start_file(ROOT_MODEL, opt).unwrap();
    z.write_all(PAINTED_MODEL.as_bytes()).unwrap();

    z.start_file(MODEL_SETTINGS, opt).unwrap();
    z.write_all(MODEL_SETTINGS_XML.as_bytes()).unwrap();

    z.start_file(PROJECT_SETTINGS, opt).unwrap();
    z.write_all(&project_settings_json(
        "Bambu Lab H2C",
        &["#FFFFFF", "#FFFF00", "#FF0000", "#0000FF"],
        &["PLA", "PLA", "PLA", "PLA"],
        None,
    ))
    .unwrap();

    z.start_file("Metadata/plate_1.gcode", opt).unwrap();
    z.write_all(b"; sliced gcode\n").unwrap();

    z.finish().unwrap().into_inner()
}

#[test]
fn convert_archives__non_identity__model_settings_changed() {
    let source = build_source_zip();
    let template = build_template_zip();
    let mut src = open_mem(&source);
    let before = read_member_bytes(&mut src, MODEL_SETTINGS).expect("before");
    let mut src = open_mem(&source);
    let mut tpl = open_mem(&template);
    let mut out_buf = Cursor::new(Vec::new());
    let map = SlotMap::parse("1=2,2=1,3=3,4=4").expect("map");
    convert_archives(
        &mut src,
        &mut tpl,
        &mut out_buf,
        &ArchiveConvertOptions::default().with_slot_map(map.clone()),
    )
    .expect("convert");
    let mut out_zip = open_mem(&out_buf.into_inner());
    let after = read_member_bytes(&mut out_zip, MODEL_SETTINGS).expect("after");
    assert_ne!(
        before, after,
        "non-identity map must rewrite model_settings"
    );
    let summary = parse_model_settings(&after).expect("parse");
    // Original: extruders 1,1,2,3 → swap 1↔2 → 2,2,1,3
    assert_eq!(summary.extruder_histogram.get(&1), Some(&1)); // was 2
    assert_eq!(summary.extruder_histogram.get(&2), Some(&2)); // was 1s
    assert_eq!(summary.extruder_histogram.get(&3), Some(&1));
}

#[test]
fn convert_archives__identity__model_settings_raw_copy() {
    // Regression: identity still byte-identical (covered also by model_settings__byte_identical)
    let source = build_source_zip();
    let template = build_template_zip();
    let mut src = open_mem(&source);
    let before = read_member_bytes(&mut src, MODEL_SETTINGS).expect("before");
    let mut src = open_mem(&source);
    let mut tpl = open_mem(&template);
    let mut out_buf = Cursor::new(Vec::new());
    convert_archives(
        &mut src,
        &mut tpl,
        &mut out_buf,
        &ArchiveConvertOptions::default(),
    )
    .expect("convert");
    let mut out_zip = open_mem(&out_buf.into_inner());
    let after = read_member_bytes(&mut out_zip, MODEL_SETTINGS).expect("after");
    assert_eq!(before, after);
}

/// Source package whose model_settings has an invalid UTF-8 element qname (DoD-7).
fn build_source_zip_invalid_qname_model_settings() -> Vec<u8> {
    let buf = Cursor::new(Vec::new());
    let mut z = ZipWriter::new(buf);
    let opt = options();

    z.start_file(ROOT_MODEL, opt).unwrap();
    z.write_all(MODEL_XML.as_bytes()).unwrap();

    // Invalid UTF-8 (0xFF) in element name — quick-xml surfaces raw bytes; parse must Error.
    z.start_file(MODEL_SETTINGS, opt).unwrap();
    z.write_all(b"<?xml version=\"1.0\"?><config><el\xFFname/><plate/></config>")
        .unwrap();

    z.start_file(PROJECT_SETTINGS, opt).unwrap();
    z.write_all(&project_settings_json(
        "Bambu Lab H2C",
        &["#AABBCC", "#DDEEFF", "#112233", "#445566"],
        &["PLA", "PLA", "PLA", "PLA"],
        None,
    ))
    .unwrap();

    z.start_file("[Content_Types].xml", opt).unwrap();
    z.write_all(b"<?xml version=\"1.0\"?><Types/>").unwrap();

    z.finish().unwrap().into_inner()
}

/// Identity convert still parses model_settings; invalid UTF-8 qnames must fail (DoD-7).
#[test]
fn convert_archives__identity_invalid_utf8_qname__errors() {
    let source = build_source_zip_invalid_qname_model_settings();
    let template = build_template_zip();
    let mut src = open_mem(&source);
    let mut tpl = open_mem(&template);
    let mut out_buf = Cursor::new(Vec::new());
    let err = convert_archives(
        &mut src,
        &mut tpl,
        &mut out_buf,
        &ArchiveConvertOptions::default(),
    )
    .expect_err("identity convert with invalid model_settings qname must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("UTF-8") || msg.contains("qname"),
        "expected UTF-8/qname error, got: {msg}"
    );
}

#[test]
fn report_markdown__contains_map_and_strip() {
    let source = build_source_zip();
    let template = build_template_zip();
    let mut src = open_mem(&source);
    let mut tpl = open_mem(&template);
    let mut out_buf = Cursor::new(Vec::new());
    let map = SlotMap::parse("1=2,2=1,3=3,4=4").expect("map");
    let report = convert_archives(
        &mut src,
        &mut tpl,
        &mut out_buf,
        &ArchiveConvertOptions::default().with_slot_map(map.clone()),
    )
    .expect("convert");
    let md = format_report_markdown(&report);
    assert!(md.contains("Slot map"), "missing Slot map section: {md}");
    assert!(
        md.contains("| 1 | 2 |") || md.contains("1") && md.contains("2"),
        "map pairs missing: {md}"
    );
    assert!(md.contains("Stripped") || md.contains("stripped"), "{md}");
    assert!(
        report
            .stripped_members
            .iter()
            .any(|m| m.ends_with(".gcode")),
        "expected gcode stripped"
    );
    assert!(report.had_gcode_stripped);
    assert!(
        md.contains("re-slice") || md.contains("re-slice"),
        "must mention re-slice: {md}"
    );
}

#[test]
fn sliced_gcode__stripped_and_warned() {
    let source = build_source_zip();
    let template = build_template_zip();
    let mut src = open_mem(&source);
    let mut tpl = open_mem(&template);
    let mut out_buf = Cursor::new(Vec::new());
    let report = convert_archives(
        &mut src,
        &mut tpl,
        &mut out_buf,
        &ArchiveConvertOptions::default(),
    )
    .expect("convert");
    assert!(report.had_gcode_stripped);
    assert!(
        report.warnings.iter().any(|w| w.contains("re-slice")),
        "warnings={:?}",
        report.warnings
    );
    let out_bytes = out_buf.into_inner();
    let mut out_zip = open_mem(&out_bytes);
    let names: Vec<String> = (0..out_zip.len())
        .map(|i| normalize_zip_path(out_zip.by_index(i).unwrap().name()))
        .collect();
    assert!(!names.iter().any(|n| n.ends_with(".gcode")));
}

#[test]
fn convert_archives__paint_remap__rewrites_triangles() {
    let source = build_source_zip_painted();
    let template = build_template_zip();
    let mut src = open_mem(&source);
    let mut tpl = open_mem(&template);
    let mut out_buf = Cursor::new(Vec::new());
    // 1→4, 16→4, 4→1 collision-safe map
    let map = SlotMap::from_pairs([(1, 4), (16, 4), (4, 1), (2, 2), (3, 3)]);
    let report = convert_archives(
        &mut src,
        &mut tpl,
        &mut out_buf,
        &ArchiveConvertOptions::default().with_slot_map(map.clone()),
    )
    .expect("convert");
    assert!(report.paint_attrs_seen >= 2);
    assert!(report.paint_attrs_rewritten >= 1);
    let mut out_zip = open_mem(&out_buf.into_inner());
    let model = read_member_bytes(&mut out_zip, ROOT_MODEL).expect("model");
    let text = String::from_utf8(model).expect("utf8");
    // paint "4" (slot1) → "1C" (slot4); "DC" (slot16) → "1C" (slot4)
    assert!(
        text.contains("paint_color=\"1C\""),
        "expected remapped paint, got: {text}"
    );
    assert!(
        !text.contains("paint_color=\"DC\""),
        "DC should be remapped"
    );
}

#[test]
fn convert_archives__swap_map__colours_reordered_when_copy_source() {
    let source = build_source_zip();
    let template = build_template_zip();
    let mut src = open_mem(&source);
    let mut tpl = open_mem(&template);
    let mut out_buf = Cursor::new(Vec::new());
    let map = SlotMap::parse("1=2,2=1,3=3,4=4").expect("map");
    convert_archives(
        &mut src,
        &mut tpl,
        &mut out_buf,
        &ArchiveConvertOptions::default()
            .with_slot_map(map.clone())
            .with_copy_source_colours(true),
    )
    .expect("convert");
    let mut out_zip = open_mem(&out_buf.into_inner());
    let ps = read_member_bytes(&mut out_zip, PROJECT_SETTINGS).expect("ps");
    let settings = parse_project_settings(&ps).expect("parse");
    // Source colours #AABBCC, #DDEEFF, #112233, #445566 with 1↔2 → DDEEFF, AABBCC, ...
    let colours = string_array_field(&settings, "filament_colour");
    assert_eq!(colours[0], "#DDEEFF");
    assert_eq!(colours[1], "#AABBCC");
    assert_eq!(colours[2], "#112233");
    assert_eq!(colours[3], "#445566");
}

#[test]
fn convert_archives__non_identity__plates_preserved() {
    let source = build_source_zip();
    let template = build_template_zip();
    let mut src = open_mem(&source);
    let mut tpl = open_mem(&template);
    let mut out_buf = Cursor::new(Vec::new());
    let map = SlotMap::parse("1=2,2=1,3=3,4=4").expect("map");
    let report = convert_archives(
        &mut src,
        &mut tpl,
        &mut out_buf,
        &ArchiveConvertOptions::default().with_slot_map(map.clone()),
    )
    .expect("convert");
    assert_eq!(report.plates, Some(2));
}

/// Synthetic source with extruder slot 5 (above ZR four-toolhead range).
fn build_source_zip_slot5_extruder() -> Vec<u8> {
    let buf = Cursor::new(Vec::new());
    let mut z = ZipWriter::new(buf);
    let opt = options();

    z.start_file(ROOT_MODEL, opt).unwrap();
    z.write_all(MODEL_XML.as_bytes()).unwrap();

    // Single object/part on extruder 5 — identity would emit 5, which is out of 1..=4.
    const MS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<config>
  <object id="1">
    <metadata key="name" value="PartSlot5"/>
    <metadata key="extruder" value="5"/>
    <part id="1" subtype="normal_part">
      <metadata key="extruder" value="5"/>
    </part>
  </object>
  <plate>
    <metadata key="plater_id" value="1"/>
  </plate>
</config>
"#;
    z.start_file(MODEL_SETTINGS, opt).unwrap();
    z.write_all(MS.as_bytes()).unwrap();

    z.start_file(PROJECT_SETTINGS, opt).unwrap();
    z.write_all(&project_settings_json(
        "Bambu Lab H2C",
        &["#111111", "#222222", "#333333", "#444444", "#555555"],
        &["PLA", "PLA", "PLA", "PLA", "PLA"],
        None,
    ))
    .unwrap();

    z.finish().unwrap().into_inner()
}

/// Identity leaves used slot 5 mapped to 5 (outside ZR 1..=4) → hard error.
#[test]
fn convert_archives__identity_used_slot5__errors() {
    let source = build_source_zip_slot5_extruder();
    let template = build_template_zip();
    let mut src = open_mem(&source);
    let mut tpl = open_mem(&template);
    let mut out_buf = Cursor::new(Vec::new());
    let err = convert_archives(
        &mut src,
        &mut tpl,
        &mut out_buf,
        &ArchiveConvertOptions::default(),
    )
    .expect_err("identity convert with used slot 5 must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("1..=4") || msg.contains("outside ZR"),
        "expected clear ZR range error, got: {msg}"
    );
    assert!(
        msg.contains('5'),
        "error should mention the offending slot: {msg}"
    );
}

/// Explicit map 5→3 brings the used slot into ZR range → success.
#[test]
fn convert_archives__map_slot5_to_3__succeeds() {
    let source = build_source_zip_slot5_extruder();
    let template = build_template_zip();
    let mut src = open_mem(&source);
    let mut tpl = open_mem(&template);
    let mut out_buf = Cursor::new(Vec::new());
    let map = SlotMap::parse("5=3").expect("map");
    let report = convert_archives(
        &mut src,
        &mut tpl,
        &mut out_buf,
        &ArchiveConvertOptions::default().with_slot_map(map.clone()),
    )
    .expect("convert");
    assert!(!report.slot_map_identity);
    let mut out_zip = open_mem(&out_buf.into_inner());
    let after = read_member_bytes(&mut out_zip, MODEL_SETTINGS).expect("model_settings");
    let summary = parse_model_settings(&after).expect("parse");
    assert_eq!(
        summary.extruder_histogram.get(&3),
        Some(&2),
        "extruder 5 must remap to toolhead 3"
    );
    assert!(
        !summary.extruder_histogram.contains_key(&5),
        "slot 5 must not remain in output histogram"
    );
}

/// Paint-only used slot 5 (code `2C`) with identity must also error.
#[test]
fn convert_archives__identity_paint_slot5__errors() {
    let source = {
        let buf = Cursor::new(Vec::new());
        let mut z = ZipWriter::new(buf);
        let opt = options();
        // paint_color "2C" = slot 5 (SLOT_CODES index 4)
        const PAINTED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<model unit="millimeter">
 <resources>
  <object id="1" type="model">
   <mesh>
    <vertices>
     <vertex x="0" y="0" z="0"/>
     <vertex x="1" y="0" z="0"/>
     <vertex x="0" y="1" z="0"/>
    </vertices>
    <triangles>
     <triangle v1="0" v2="1" v3="2" paint_color="2C"/>
    </triangles>
   </mesh>
  </object>
 </resources>
</model>
"#;
        z.start_file(ROOT_MODEL, opt).unwrap();
        z.write_all(PAINTED.as_bytes()).unwrap();
        // Extruders only in 1..=3 so failure must come from paint slot 5.
        z.start_file(MODEL_SETTINGS, opt).unwrap();
        z.write_all(MODEL_SETTINGS_XML.as_bytes()).unwrap();
        z.start_file(PROJECT_SETTINGS, opt).unwrap();
        z.write_all(&project_settings_json(
            "Bambu Lab H2C",
            &["#FFFFFF", "#FFFF00", "#FF0000", "#0000FF"],
            &["PLA", "PLA", "PLA", "PLA"],
            None,
        ))
        .unwrap();
        z.finish().unwrap().into_inner()
    };
    let template = build_template_zip();
    let mut src = open_mem(&source);
    let mut tpl = open_mem(&template);
    let mut out_buf = Cursor::new(Vec::new());
    let err = convert_archives(
        &mut src,
        &mut tpl,
        &mut out_buf,
        &ArchiveConvertOptions::default(),
    )
    .expect_err("identity convert with paint slot 5 must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("1..=4") || msg.contains("outside ZR"),
        "expected clear ZR range error, got: {msg}"
    );
}

// --- 0003: edge cases / hardening ---

fn build_source_zip_large_bed() -> Vec<u8> {
    let buf = Cursor::new(Vec::new());
    let mut z = ZipWriter::new(buf);
    let opt = options();
    z.start_file(ROOT_MODEL, opt).unwrap();
    z.write_all(MODEL_XML.as_bytes()).unwrap();
    z.start_file(MODEL_SETTINGS, opt).unwrap();
    z.write_all(MODEL_SETTINGS_XML.as_bytes()).unwrap();
    z.start_file(PROJECT_SETTINGS, opt).unwrap();
    z.write_all(&project_settings_json_ex(
        "Bambu Lab H2C",
        &["#AABBCC", "#DDEEFF", "#112233", "#445566"],
        &["PLA", "PLA", "PLA", "PLA"],
        None,
        Some(&["0x0", "330x0", "330x320", "0x320"]),
        Some("; machine: H2C\nG28 ; home\n"),
    ))
    .unwrap();
    z.start_file("Metadata/plate_1.gcode", opt).unwrap();
    z.write_all(b"; fake gcode\n").unwrap();
    // Member name with space — Relationship Target may percent-encode it as %20 (C3).
    z.start_file("Metadata/my plate.gcode", opt).unwrap();
    z.write_all(b"; spaced gcode\n").unwrap();
    z.start_file(CONTENT_TYPES, opt).unwrap();
    z.write_all(
        br#"<?xml version="1.0"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/>
  <Default Extension="gcode" ContentType="text/plain"/>
  <Override PartName="/Metadata/plate_1.gcode" ContentType="text/plain"/>
  <Override PartName="/Metadata/my%20plate.gcode" ContentType="text/plain"/>
  <Override PartName="/3D/3dmodel.model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/>
</Types>
"#,
    )
    .unwrap();
    z.start_file("_rels/.rels", opt).unwrap();
    z.write_all(
        br#"<?xml version="1.0"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel" Target="/3D/3dmodel.model"/>
  <Relationship Id="rId2" Type="http://example/gcode" Target="/Metadata/plate_1.gcode"/>
  <Relationship Id="rId3" Type="http://example/gcode" Target="Metadata\plate_1.gcode"/>
  <Relationship Id="rId4" Type="http://example/gcode" Target="/Metadata/my%20plate.gcode"/>
</Relationships>
"#,
    )
    .unwrap();
    z.finish().unwrap().into_inner()
}

fn build_template_zip_with_gcode_clean() -> Vec<u8> {
    let buf = Cursor::new(Vec::new());
    let mut z = ZipWriter::new(buf);
    let opt = options();
    z.start_file(ROOT_MODEL, opt).unwrap();
    z.write_all(
        br#"<?xml version="1.0"?><model><metadata name="Application">BambuStudio-2.3.1</metadata></model>"#,
    )
    .unwrap();
    z.start_file(PROJECT_SETTINGS, opt).unwrap();
    z.write_all(&project_settings_json_ex(
        "WonderMaker ZR Ultra S",
        &["#FFFFFF", "#FFFF00", "#FF0000", "#0000FF"],
        &["PETG", "PETG", "PETG", "PETG"],
        None,
        Some(&["0x0", "300x0", "300x270", "0x270"]),
        Some("; WonderMaker ZR start\nG28\n"),
    ))
    .unwrap();
    z.start_file(MODEL_SETTINGS, opt).unwrap();
    z.write_all(br#"<?xml version="1.0"?><config><plate/></config>"#)
        .unwrap();
    z.start_file(CONTENT_TYPES, opt).unwrap();
    z.write_all(
        br#"<?xml version="1.0"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/>
</Types>
"#,
    )
    .unwrap();
    z.start_file("_rels/.rels", opt).unwrap();
    z.write_all(
        br#"<?xml version="1.0"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel" Target="/3D/3dmodel.model"/>
</Relationships>
"#,
    )
    .unwrap();
    z.finish().unwrap().into_inner()
}

/// Geometry-only package: no project_settings, no model_settings.
fn build_geometry_only_source() -> Vec<u8> {
    let buf = Cursor::new(Vec::new());
    let mut z = ZipWriter::new(buf);
    let opt = options();
    z.start_file(ROOT_MODEL, opt).unwrap();
    z.write_all(MODEL_XML.as_bytes()).unwrap();
    z.start_file("3D/Objects/object_1.model", opt).unwrap();
    z.write_all(NESTED_OBJECT.as_bytes()).unwrap();
    z.start_file(CONTENT_TYPES, opt).unwrap();
    z.write_all(
        br#"<?xml version="1.0"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/>
  <Override PartName="/3D/Objects/object_1.model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/>
</Types>
"#,
    )
    .unwrap();
    z.start_file("_rels/.rels", opt).unwrap();
    z.write_all(
        br#"<?xml version="1.0"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel" Target="/3D/3dmodel.model"/>
</Relationships>
"#,
    )
    .unwrap();
    z.finish().unwrap().into_inner()
}

/// Empty-ish template shell without model Default (S2 C1 merge stress).
fn build_empty_shell_template() -> Vec<u8> {
    let buf = Cursor::new(Vec::new());
    let mut z = ZipWriter::new(buf);
    let opt = options();
    z.start_file(ROOT_MODEL, opt).unwrap();
    z.write_all(br#"<?xml version="1.0"?><model></model>"#)
        .unwrap();
    z.start_file(PROJECT_SETTINGS, opt).unwrap();
    z.write_all(&project_settings_json_ex(
        "WonderMaker ZR Ultra S",
        &["#FFFFFF", "#FFFF00", "#FF0000", "#0000FF"],
        &["PETG", "PETG", "PETG", "PETG"],
        None,
        None,
        Some("; WonderMaker clean start\n"),
    ))
    .unwrap();
    z.start_file(MODEL_SETTINGS, opt).unwrap();
    z.write_all(br#"<?xml version="1.0"?><config><plate/></config>"#)
        .unwrap();
    z.start_file(CONTENT_TYPES, opt).unwrap();
    // Intentionally no model Default — S2 must merge/synthesize.
    z.write_all(
        br#"<?xml version="1.0"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
</Types>
"#,
    )
    .unwrap();
    z.start_file("_rels/.rels", opt).unwrap();
    z.write_all(
        br#"<?xml version="1.0"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
</Relationships>
"#,
    )
    .unwrap();
    z.finish().unwrap().into_inner()
}

fn build_source_zip_5_extruders() -> Vec<u8> {
    let buf = Cursor::new(Vec::new());
    let mut z = ZipWriter::new(buf);
    let opt = options();
    z.start_file(ROOT_MODEL, opt).unwrap();
    z.write_all(MODEL_XML.as_bytes()).unwrap();
    const MS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<config>
  <object id="1">
    <metadata key="extruder" value="1"/>
    <part id="1" subtype="normal_part"><metadata key="extruder" value="1"/></part>
    <part id="2" subtype="normal_part"><metadata key="extruder" value="2"/></part>
    <part id="3" subtype="normal_part"><metadata key="extruder" value="3"/></part>
    <part id="4" subtype="normal_part"><metadata key="extruder" value="4"/></part>
    <part id="5" subtype="normal_part"><metadata key="extruder" value="5"/></part>
  </object>
  <plate><metadata key="plater_id" value="1"/></plate>
</config>
"#;
    z.start_file(MODEL_SETTINGS, opt).unwrap();
    z.write_all(MS.as_bytes()).unwrap();
    z.start_file(PROJECT_SETTINGS, opt).unwrap();
    z.write_all(&project_settings_json(
        "Bambu Lab H2C",
        &["#111111", "#222222", "#333333", "#444444", "#555555"],
        &["PLA", "PLA", "PLA", "PLA", "PLA"],
        None,
    ))
    .unwrap();
    z.finish().unwrap().into_inner()
}

#[test]
fn bed__polygon_max_bounds() {
    let s = json!({
        "printable_area": ["0x0", "250x10", "330x50", "100x320", "0x100"]
    });
    assert_eq!(bed_size_mm(&s), Some((330.0, 320.0)));
}

#[test]
fn bed__source_larger_than_template__warns() {
    let source = build_source_zip_large_bed();
    let template = build_template_zip_with_gcode_clean();
    let mut src = open_mem(&source);
    let mut tpl = open_mem(&template);
    let mut out_buf = Cursor::new(Vec::new());
    let report = convert_archives(
        &mut src,
        &mut tpl,
        &mut out_buf,
        &ArchiveConvertOptions::default(),
    )
    .expect("convert");
    let expected = bed_compare_message((330.0, 320.0), (300.0, 270.0));
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.contains(&expected) || w.contains("330") && w.contains("300")),
        "expected bed warning, got {:?}",
        report.warnings
    );
}

#[test]
fn bed__strict__errors() {
    let source = build_source_zip_large_bed();
    let template = build_template_zip_with_gcode_clean();
    let mut src = open_mem(&source);
    let mut tpl = open_mem(&template);
    let mut out_buf = Cursor::new(Vec::new());
    let opts = ArchiveConvertOptions {
        strict_bed: true,
        ..ArchiveConvertOptions::default()
    };
    let err = convert_archives(&mut src, &mut tpl, &mut out_buf, &opts).expect_err("strict bed");
    let msg = err.to_string();
    assert!(
        msg.contains("Source bed") && msg.contains("vs template"),
        "got {msg}"
    );
}

#[test]
fn bed__equal_silent() {
    let source = build_source_zip(); // 300×270
    let template = build_template_zip(); // 300×270
    let mut src = open_mem(&source);
    let mut tpl = open_mem(&template);
    let mut out_buf = Cursor::new(Vec::new());
    let report = convert_archives(
        &mut src,
        &mut tpl,
        &mut out_buf,
        &ArchiveConvertOptions::default(),
    )
    .expect("convert");
    assert!(
        !report
            .warnings
            .iter()
            .any(|w| w.contains("Source bed") && w.contains("vs template")),
        "equal beds must not warn: {:?}",
        report.warnings
    );
    assert!(!bed_source_exceeds_template(
        (300.0, 270.0),
        (300.0, 270.0),
        BED_COMPARE_EPS_MM
    ));
}

#[test]
fn used_slots_5__identity__errors() {
    let source = build_source_zip_5_extruders();
    let template = build_template_zip();
    let mut src = open_mem(&source);
    let mut tpl = open_mem(&template);
    let mut out_buf = Cursor::new(Vec::new());
    let err = convert_archives(
        &mut src,
        &mut tpl,
        &mut out_buf,
        &ArchiveConvertOptions::default(),
    )
    .expect_err("5 used identity");
    let msg = err.to_string();
    assert!(msg.contains("1..=4"), "{msg}");
    assert!(msg.contains("Used sources") || msg.contains("5"), "{msg}");
    assert!(
        msg.contains("--map") || msg.contains("map"),
        "actionable map hint missing: {msg}"
    );
}

#[test]
fn used_slots_5__map_merge_to_4__ok() {
    let source = build_source_zip_5_extruders();
    let template = build_template_zip();
    let mut src = open_mem(&source);
    let mut tpl = open_mem(&template);
    let mut out_buf = Cursor::new(Vec::new());
    let map = SlotMap::parse("1=1,2=2,3=3,4=4,5=4").expect("map");
    let report = convert_archives(
        &mut src,
        &mut tpl,
        &mut out_buf,
        &ArchiveConvertOptions::default().with_slot_map(map),
    )
    .expect("convert");
    assert_eq!(report.strategy.as_str(), "S1");
    let mut out_zip = open_mem(&out_buf.into_inner());
    let ms = read_member_bytes(&mut out_zip, MODEL_SETTINGS).expect("ms");
    let summary = parse_model_settings(&ms).expect("parse");
    assert!(
        !summary.extruder_histogram.contains_key(&5),
        "slot 5 must merge away"
    );
    assert!(
        summary.extruder_histogram.get(&4).copied().unwrap_or(0) >= 2,
        "dest 4 should receive merge: {:?}",
        summary.extruder_histogram
    );
}

#[test]
fn opc__normalize_targets__slash_and_pct20() {
    assert_eq!(
        normalize_opc_part_name("/Metadata/plate_1.gcode"),
        "Metadata/plate_1.gcode"
    );
    assert_eq!(
        normalize_opc_part_name(r"Metadata\plate_1.gcode"),
        "Metadata/plate_1.gcode"
    );
    assert_eq!(
        normalize_opc_part_name("Metadata/my%20plate.gcode"),
        "Metadata/my plate.gcode"
    );
}

#[test]
fn opc__strip_gcode__removes_override_and_rel() {
    let source = build_source_zip_large_bed();
    let template = build_template_zip_with_gcode_clean();
    let mut src = open_mem(&source);
    let mut tpl = open_mem(&template);
    let mut out_buf = Cursor::new(Vec::new());
    let report = convert_archives(
        &mut src,
        &mut tpl,
        &mut out_buf,
        &ArchiveConvertOptions::default(),
    )
    .expect("convert");
    assert!(report.opc_reconciled || report.had_gcode_stripped);
    let mut out_zip = open_mem(&out_buf.into_inner());
    let ct = read_member_bytes(&mut out_zip, CONTENT_TYPES).expect("ct");
    let ct_text = String::from_utf8_lossy(&ct);
    assert!(
        !ct_text.contains("plate_1.gcode"),
        "Override for stripped gcode must drop: {ct_text}"
    );
    let rels = read_member_bytes(&mut out_zip, "_rels/.rels").expect("rels");
    let rels_text = String::from_utf8_lossy(&rels);
    assert!(
        !rels_text.contains("plate_1.gcode"),
        "Relationship Target for gcode must drop: {rels_text}"
    );
    assert!(
        !rels_text.contains("my%20plate") && !rels_text.contains("my plate"),
        "percent-decoded gcode target must drop: {rels_text}"
    );
}

#[test]
fn s2__geometry_only__gets_template_printer() {
    let source = build_geometry_only_source();
    let template = build_empty_shell_template();
    let mut src = open_mem(&source);
    let mut tpl = open_mem(&template);
    let mut out_buf = Cursor::new(Vec::new());
    let report = convert_archives(
        &mut src,
        &mut tpl,
        &mut out_buf,
        &ArchiveConvertOptions::default(), // Auto → S2
    )
    .expect("s2 convert");
    assert_eq!(report.strategy.as_str(), "S2");
    assert!(
        report
            .output_printer
            .as_deref()
            .is_some_and(|p| p.contains("WonderMaker")),
        "printer={:?}",
        report.output_printer
    );
    let mut out_zip = open_mem(&out_buf.into_inner());
    let ps = read_member_bytes(&mut out_zip, PROJECT_SETTINGS).expect("ps");
    let settings = parse_project_settings(&ps).expect("parse");
    assert!(
        string_field(&settings, "printer_model")
            .as_deref()
            .is_some_and(|p| p.contains("WonderMaker"))
    );
    // Geometry present
    let root = read_member_bytes(&mut out_zip, ROOT_MODEL).expect("root");
    assert!(!root.is_empty());
    let nested = read_member_bytes(&mut out_zip, "3D/Objects/object_1.model").expect("nested");
    assert_eq!(nested, NESTED_OBJECT.as_bytes());
    // C1 / empty-shell: package rels must target root 3dmodel after ensure_root_model_relationship
    let rels = read_member_bytes(&mut out_zip, PACKAGE_RELS).expect("package rels");
    let rels_text = String::from_utf8_lossy(&rels);
    assert!(
        rels_text.contains("3dmodel.model") || rels_text.contains("3D/3dmodel.model"),
        "S2 empty template must gain root model relationship: {rels_text}"
    );
}

#[test]
fn s2__inject_merges_content_types_for_model() {
    let source = build_geometry_only_source();
    let template = build_empty_shell_template();
    let mut src = open_mem(&source);
    let mut tpl = open_mem(&template);
    let mut out_buf = Cursor::new(Vec::new());
    convert_archives(
        &mut src,
        &mut tpl,
        &mut out_buf,
        &ArchiveConvertOptions {
            strategy: ConvertStrategy::S2,
            ..ArchiveConvertOptions::default()
        },
    )
    .expect("s2");
    let mut out_zip = open_mem(&out_buf.into_inner());
    let ct = read_member_bytes(&mut out_zip, CONTENT_TYPES).expect("ct");
    let text = String::from_utf8_lossy(&ct);
    assert!(
        text.contains("Extension=\"model\"") || text.contains("Extension='model'"),
        "Content_Types must cover .model: {text}"
    );
    assert!(
        text.contains("object_1.model") || text.contains("3dmodel.model"),
        "should mention model parts: {text}"
    );
}

#[test]
fn s2__no_model_settings__synthesizes_plate_objects() {
    let source = build_geometry_only_source();
    let template = build_empty_shell_template();
    let mut src = open_mem(&source);
    let mut tpl = open_mem(&template);
    let mut out_buf = Cursor::new(Vec::new());
    convert_archives(
        &mut src,
        &mut tpl,
        &mut out_buf,
        &ArchiveConvertOptions::default(),
    )
    .expect("s2");
    let mut out_zip = open_mem(&out_buf.into_inner());
    let ms = read_member_bytes(&mut out_zip, MODEL_SETTINGS).expect("ms");
    let text = String::from_utf8_lossy(&ms);
    assert!(text.contains("object"), "synthesized ms: {text}");
    assert!(
        text.contains("plate") || text.contains("plater_id"),
        "{text}"
    );
    let summary = parse_model_settings(&ms).expect("parse");
    assert!(summary.plate_count >= 1);
    assert!(
        !summary.extruder_histogram.is_empty() || text.contains("object_id"),
        "must reference objects: hist={:?} text={text}",
        summary.extruder_histogram
    );
}

#[test]
fn s2__auto_with_project_settings__still_s1() {
    let source = build_source_zip();
    let template = build_template_zip();
    let mut src = open_mem(&source);
    let mut tpl = open_mem(&template);
    let mut out_buf = Cursor::new(Vec::new());
    let report = convert_archives(
        &mut src,
        &mut tpl,
        &mut out_buf,
        &ArchiveConvertOptions {
            strategy: ConvertStrategy::Auto,
            ..ArchiveConvertOptions::default()
        },
    )
    .expect("convert");
    assert_eq!(report.strategy.as_str(), "S1");
    // Default: keep template colours (copy_source_colours = false).
    assert!(!report.colours_patched);
}

#[test]
fn s2__force_s1_on_geometry_only__errors() {
    let source = build_geometry_only_source();
    let template = build_empty_shell_template();
    let mut src = open_mem(&source);
    let mut tpl = open_mem(&template);
    let mut out_buf = Cursor::new(Vec::new());
    let err = convert_archives(
        &mut src,
        &mut tpl,
        &mut out_buf,
        &ArchiveConvertOptions {
            strategy: ConvertStrategy::S1,
            ..ArchiveConvertOptions::default()
        },
    )
    .expect_err("force S1");
    let msg = err.to_string();
    assert!(
        msg.contains("S1") && msg.contains("project_settings"),
        "got {msg}"
    );
}

#[test]
fn golden__no_h2c_start_gcode_in_output_settings() {
    let source = build_source_zip_large_bed();
    let template = build_template_zip_with_gcode_clean();
    let mut src = open_mem(&source);
    let mut tpl = open_mem(&template);
    let mut out_buf = Cursor::new(Vec::new());
    let report = convert_archives(
        &mut src,
        &mut tpl,
        &mut out_buf,
        &ArchiveConvertOptions::default(),
    )
    .expect("convert");
    assert!(
        report
            .output_printer
            .as_deref()
            .is_some_and(|p| p.contains("WonderMaker"))
    );
    let mut out_zip = open_mem(&out_buf.into_inner());
    let ps = read_member_bytes(&mut out_zip, PROJECT_SETTINGS).expect("ps");
    let settings = parse_project_settings(&ps).expect("parse");
    let start = string_field(&settings, "machine_start_gcode").unwrap_or_default();
    assert!(
        !start.contains("H2C"),
        "output machine_start_gcode must not contain H2C: {start}"
    );
    assert!(
        string_field(&settings, "printer_model")
            .as_deref()
            .is_some_and(|p| p.contains("WonderMaker"))
    );
    // No gcode members
    let names: Vec<String> = (0..out_zip.len())
        .map(|i| normalize_zip_path(out_zip.by_index(i).unwrap().name()))
        .collect();
    assert!(!names.iter().any(|n| n.ends_with(".gcode")));
}

#[test]
fn truncated_zip__errors() {
    // Truncate a valid ZIP in the middle of the central directory / file data.
    let good = build_source_zip();
    assert!(good.len() > 64);
    let truncated = &good[..good.len() / 3];
    let result = ZipArchive::new(Cursor::new(truncated.to_vec()));
    assert!(
        result.is_err(),
        "truncated ZIP must fail to open or be unusable"
    );
    // Also exercise convert path if open somehow succeeds on partial (belt-and-suspenders).
    if let Ok(mut archive) = ZipArchive::new(Cursor::new(truncated.to_vec())) {
        let template = build_template_zip();
        let mut tpl = open_mem(&template);
        let mut out_buf = Cursor::new(Vec::new());
        let err = convert_archives(
            &mut archive,
            &mut tpl,
            &mut out_buf,
            &ArchiveConvertOptions::default(),
        );
        assert!(err.is_err(), "convert on truncated must error");
    }
}

#[test]
fn unknown_subtype__warns_non_fatal() {
    let source = {
        let buf = Cursor::new(Vec::new());
        let mut z = ZipWriter::new(buf);
        let opt = options();
        z.start_file(ROOT_MODEL, opt).unwrap();
        z.write_all(MODEL_XML.as_bytes()).unwrap();
        const MS: &str = r#"<?xml version="1.0"?>
<config>
  <object id="1">
    <metadata key="extruder" value="1"/>
    <part id="1" subtype="normal_part"><metadata key="extruder" value="1"/></part>
    <part id="2" subtype="mystery_feature_x"><metadata key="extruder" value="2"/></part>
  </object>
  <plate><metadata key="plater_id" value="1"/></plate>
</config>
"#;
        z.start_file(MODEL_SETTINGS, opt).unwrap();
        z.write_all(MS.as_bytes()).unwrap();
        z.start_file(PROJECT_SETTINGS, opt).unwrap();
        z.write_all(&project_settings_json(
            "Bambu Lab H2C",
            &["#AABBCC", "#DDEEFF"],
            &["PLA", "PLA"],
            None,
        ))
        .unwrap();
        z.finish().unwrap().into_inner()
    };
    let template = build_template_zip();
    let mut src = open_mem(&source);
    let mut tpl = open_mem(&template);
    let mut out_buf = Cursor::new(Vec::new());
    let report = convert_archives(
        &mut src,
        &mut tpl,
        &mut out_buf,
        &ArchiveConvertOptions::default(),
    )
    .expect("convert must succeed with warn");
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.contains("mystery_feature_x")),
        "warnings={:?}",
        report.warnings
    );
}
