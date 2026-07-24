//! Thin CLI over `wondermaker_3mf_core`.

use anyhow::{Context, Result, bail};
use camino::Utf8PathBuf;
use clap::{Parser, Subcommand};
use wondermaker_3mf_core::{
    ConvertOptions, SlotMap, analyze, convert, default_output_path, format_analysis_human,
    format_report_human,
};

#[derive(Parser, Debug)]
#[command(
    name = "wondermaker_3mf_cli",
    about = "Analyze and convert MakerWorld/Bambu project 3MF → Wonderprint ZR Ultra-S via settings graft",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Analyze a project 3MF (printer, plates, filaments, paint, extruders).
    Analyze {
        /// Input .3mf path
        #[arg(long)]
        input: Utf8PathBuf,
    },
    /// Convert via S1 settings graft (template project_settings + source colours).
    Convert {
        /// Source MakerWorld/Bambu project .3mf
        #[arg(long)]
        input: Utf8PathBuf,
        /// Wonderprint ZR template .3mf (donor project_settings)
        #[arg(long)]
        template: Utf8PathBuf,
        /// Output path (default: <input-stem>-zr-ultra-s.3mf beside input)
        #[arg(long)]
        output: Option<Utf8PathBuf>,
        /// Slot map SOURCE=DEST pairs, e.g. `1=2,2=1,3=3,4=4` (default: identity)
        #[arg(long)]
        map: Option<String>,
        /// Markdown report path (default: <output-stem>-conversion-report.md)
        #[arg(long)]
        report: Option<Utf8PathBuf>,
        /// Do not write a markdown conversion report
        #[arg(long, default_value_t = false)]
        no_report: bool,
        /// Do not copy filament_type labels from source (keep template types)
        #[arg(long, default_value_t = false)]
        keep_template_filament_type: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Analyze { input } => cmd_analyze(input),
        Commands::Convert {
            input,
            template,
            output,
            map,
            report,
            no_report,
            keep_template_filament_type,
        } => cmd_convert(
            input,
            template,
            output,
            map,
            report,
            no_report,
            keep_template_filament_type,
        ),
    }
}

fn cmd_analyze(input: Utf8PathBuf) -> Result<()> {
    if !input.exists() {
        bail!("input not found: {input}");
    }
    let analysis = analyze(&input).with_context(|| format!("analyze failed for {input}"))?;
    print!("{}", format_analysis_human(&analysis));
    Ok(())
}

fn cmd_convert(
    input: Utf8PathBuf,
    template: Utf8PathBuf,
    output: Option<Utf8PathBuf>,
    map: Option<String>,
    report: Option<Utf8PathBuf>,
    no_report: bool,
    keep_template_filament_type: bool,
) -> Result<()> {
    if !input.exists() {
        bail!("input not found: {input}");
    }
    if !template.exists() {
        bail!("template not found: {template}");
    }
    let output = output.unwrap_or_else(|| default_output_path(&input));
    let slot_map = match map {
        Some(spec) => SlotMap::parse(&spec).with_context(|| format!("invalid --map: {spec}"))?,
        None => SlotMap::identity(),
    };

    let mut opts = ConvertOptions::new(&input, &template, &output);
    opts.slot_map = slot_map;
    opts.copy_filament_type = !keep_template_filament_type;
    opts.write_report = !no_report;
    opts.report_path = report;

    let report = convert(&opts).with_context(|| {
        format!("convert failed (input={input}, template={template}, output={output})")
    })?;
    print!("{}", format_report_human(&report));
    Ok(())
}
