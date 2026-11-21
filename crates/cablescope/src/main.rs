//! Version portable de CableScope.

// Version publiée sous Windows : application graphique, sans console au
// double-clic (les modes texte se rattachent au terminal appelant).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod export;
mod gui;
mod platform;
mod prefs;
mod probe;
mod session;
mod text;
mod tui;

use std::io::IsTerminal;
use std::process::ExitCode;

use cablescope_model::Report;

const USAGE: &str = "\
CableScope : audit des ports, câbles et appareils USB

Usage : cablescope [options]

Sans option (double-clic) : interface graphique dans une fenêtre
(analyse, test de câble, diagnostic approfondi, export).

Options :
  --tui              Interface plein écran dans la console
  --gui              Interface graphique (avec --input : affiche ce rapport)
  --no-window        Interface graphique sans ouvrir de fenêtre (affiche l'adresse)
  --json             Écrit le rapport JSON (à partager ou à ouvrir sur le site)
  --input <fichier>  Audite un rapport JSON existant au lieu de la machine
  --details          Vue détaillée (arbre par contrôleur) au lieu de la vue par prise
  --deep             Ouvre directement le diagnostic approfondi (administrateur)
  --include-serials  Inclut les numéros de série (exclus par défaut)
  -V, --version      Affiche la version
  -h, --help         Affiche cette aide";

struct Args {
    json: bool,
    tui: bool,
    gui: bool,
    no_window: bool,
    details: bool,
    deep: bool,
    input: Option<String>,
    include_serials: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        json: false,
        tui: false,
        gui: false,
        no_window: false,
        details: false,
        deep: false,
        input: None,
        include_serials: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--json" => args.json = true,
            "--tui" => args.tui = true,
            "--gui" => args.gui = true,
            "--no-window" => {
                args.gui = true;
                args.no_window = true;
            }
            "--deep" => args.deep = true,
            "--details" => args.details = true,
            "--include-serials" => args.include_serials = true,
            "--input" => args.input = Some(it.next().ok_or("--input attend un fichier")?),
            "-V" | "--version" => {
                platform::attach_console();
                println!(
                    "cablescope {} ({})",
                    env!("CARGO_PKG_VERSION"),
                    env!("CABLESCOPE_BUILD")
                );
                std::process::exit(0);
            }
            "-h" | "--help" => {
                platform::attach_console();
                println!("{USAGE}");
                std::process::exit(0);
            }
            other => return Err(format!("option inconnue : {other}\n\n{USAGE}")),
        }
    }
    Ok(args)
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            platform::attach_console();
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let options = cablescope_collect::Options {
        include_serials: args.include_serials,
        ..Default::default()
    };

    if args.tui {
        platform::own_console();
        return match tui::run(&options, args.deep) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("Erreur : {e}");
                ExitCode::FAILURE
            }
        };
    }

    // Par défaut : interface graphique.
    let text_request = args.json || args.details || (args.input.is_some() && !args.gui);
    if !text_request && (cfg!(windows) || args.gui || wants_window()) {
        let fixed_report = match args.input.as_deref().map(read_report).transpose() {
            Ok(r) => r,
            Err(e) => {
                platform::message(&format!("Rapport illisible : {e}"));
                return ExitCode::FAILURE;
            }
        };
        return match gui::run(gui::Config {
            options,
            start_deep: args.deep,
            open_window: !args.no_window,
            fixed_report,
        }) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                platform::message(&format!("CableScope n'a pas pu démarrer : {e}"));
                ExitCode::FAILURE
            }
        };
    }

    platform::attach_console();
    let report = match &args.input {
        Some(path) => read_report(path),
        None => cablescope_collect::collect(&options).map_err(|e| e.to_string()),
    };
    let report = match report {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Erreur : {e}");
            return ExitCode::FAILURE;
        }
    };

    if args.json {
        println!("{}", to_json(&report));
        return ExitCode::SUCCESS;
    }
    print!("{}", text::plain(&report, args.details));
    ExitCode::SUCCESS
}

/// Ailleurs que sous Windows, l'interface graphique s'ouvre depuis un
/// terminal interactif, ou au double-clic dans une session graphique (pas de
/// terminal en entrée, mais un affichage). Une sortie redirigée depuis un
/// terminal (`cablescope | less`, `> rapport.txt`) et une session sans
/// affichage (SSH) gardent le rapport texte.
fn wants_window() -> bool {
    let stdin = std::io::stdin().is_terminal();
    let stdout = std::io::stdout().is_terminal();
    let display = ["DISPLAY", "WAYLAND_DISPLAY"]
        .iter()
        .any(|v| std::env::var_os(v).is_some_and(|d| !d.is_empty()));
    window_wanted(stdin, stdout, display)
}

fn window_wanted(stdin_terminal: bool, stdout_terminal: bool, display: bool) -> bool {
    (stdin_terminal && stdout_terminal) || (!stdin_terminal && display)
}

fn read_report(path: &str) -> Result<Report, String> {
    std::fs::read_to_string(path)
        .map_err(|e| e.to_string())
        .and_then(|s| serde_json::from_str::<Report>(&s).map_err(|e| e.to_string()))
}

fn to_json(report: &Report) -> String {
    serde_json::to_string_pretty(report).expect("rapport sérialisable")
}

#[cfg(test)]
mod tests {
    use super::window_wanted;

    #[test]
    fn window_or_text_report() {
        // Terminal interactif.
        assert!(window_wanted(true, true, false));
        // Double-clic dans une session graphique : ni terminal, mais un affichage.
        assert!(window_wanted(false, false, true));
        // Sortie redirigée depuis un terminal : rapport texte.
        assert!(!window_wanted(true, false, true));
        // Session sans affichage (SSH, service) sans terminal : rapport texte.
        assert!(!window_wanted(false, false, false));
    }
}
