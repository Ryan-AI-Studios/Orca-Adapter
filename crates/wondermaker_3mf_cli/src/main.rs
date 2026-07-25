//! Thin CLI over `wondermaker_3mf_core`.

use anyhow::{Context, Result, bail};
use camino::Utf8PathBuf;
use clap::{Parser, Subcommand};
use wondermaker_3mf_core::{
    ConvertOptions, ConvertStrategy, SlotMap, analyze, convert, default_output_path,
    format_analysis_human, format_report_human,
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
    /// Convert via S1 settings graft or S2 template shell (see --strategy).
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
        /// Error when source bed exceeds template bed (either dimension, eps ~0.5 mm)
        #[arg(long, default_value_t = false)]
        strict_bed: bool,
        /// Conversion strategy: auto (default), s1 (settings graft), s2 (template shell)
        #[arg(long, default_value = "auto", value_parser = parse_strategy)]
        strategy: ConvertStrategy,
    },
}

fn parse_strategy(s: &str) -> std::result::Result<ConvertStrategy, String> {
    s.parse::<ConvertStrategy>().map_err(|e| e.to_string())
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
            strict_bed,
            strategy,
        } => {
            if !input.exists() {
                bail!("input not found: {input}");
            }
            if !template.exists() {
                bail!("template not found: {template}");
            }
            let output = output.unwrap_or_else(|| default_output_path(&input));
            let slot_map = match map {
                Some(spec) => {
                    SlotMap::parse(&spec).with_context(|| format!("invalid --map: {spec}"))?
                }
                None => SlotMap::identity(),
            };

            let opts = ConvertOptions {
                source: input.clone(),
                template: template.clone(),
                output: output.clone(),
                slot_map,
                copy_filament_type: !keep_template_filament_type,
                write_report: !no_report,
                report_path: report,
                strict_bed,
                strategy,
            };

            let report = convert(&opts).with_context(|| {
                format!("convert failed (input={input}, template={template}, output={output})")
            })?;
            print!("{}", format_report_human(&report));
            Ok(())
        }
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
