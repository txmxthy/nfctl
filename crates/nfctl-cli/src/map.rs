//! `nfctl map`: every command and option in one tree, the whole surface at once.
//! `--help` shows one slice at a time; this is the view for pruning.

use std::fmt::Write as _;

use clap::{Arg, ArgAction, Command};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ArgKind {
    Positional,
    Flag,
    Option,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ArgInfo {
    pub name: String,
    pub kind: ArgKind,
    pub value: Option<String>,
    pub default: Option<String>,
    pub choices: Vec<String>,
    pub help: String,
    pub global: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Node {
    pub name: String,
    pub aliases: Vec<String>,
    pub help: String,
    pub args: Vec<ArgInfo>,
    pub children: Vec<Node>,
}

fn describe_arg(a: &Arg) -> ArgInfo {
    let (name, kind) = match (a.get_long(), a.get_short()) {
        (Some(l), Some(s)) => (format!("-{s}/--{l}"), ArgKind::Option),
        (Some(l), None) => (format!("--{l}"), ArgKind::Option),
        (None, Some(s)) => (format!("-{s}"), ArgKind::Option),
        (None, None) => (a.get_id().to_string(), ArgKind::Positional),
    };
    let takes_value = a.get_action().takes_values();
    let kind = match kind {
        ArgKind::Option if !takes_value => ArgKind::Flag,
        k => k,
    };
    let value = takes_value.then(|| {
        a.get_value_names().and_then(|v| v.first()).map_or_else(
            || a.get_id().to_string().to_uppercase(),
            ToString::to_string,
        )
    });
    let default = match a.get_action() {
        ArgAction::SetTrue
        | ArgAction::SetFalse
        | ArgAction::Count
        | ArgAction::Help
        | ArgAction::Version => None,
        _ => {
            let d: Vec<String> = a
                .get_default_values()
                .iter()
                .map(|v| v.to_string_lossy().into_owned())
                .collect();
            (!d.is_empty()).then(|| d.join(","))
        }
    };
    let choices = if kind == ArgKind::Flag {
        Vec::new()
    } else {
        a.get_possible_values()
            .iter()
            .filter(|p| !p.is_hide_set())
            .map(|p| p.get_name().to_owned())
            .collect()
    };
    ArgInfo {
        name,
        kind,
        value,
        default,
        choices,
        help: a.get_help().map(ToString::to_string).unwrap_or_default(),
        global: a.is_global_set(),
    }
}

/// Describe a command. Global arguments are listed on the root only, since
/// clap repeats them on every subcommand.
pub fn describe(cmd: &Command) -> Node {
    describe_at(cmd, true)
}

fn describe_at(cmd: &Command, root: bool) -> Node {
    let args = cmd
        .get_arguments()
        .filter(|a| !matches!(a.get_action(), ArgAction::Help | ArgAction::Version))
        .filter(|a| root || !a.is_global_set())
        .map(describe_arg)
        .collect();
    let children = cmd
        .get_subcommands()
        .filter(|c| !c.is_hide_set())
        .map(|c| describe_at(c, false))
        .collect();
    Node {
        name: cmd.get_name().to_owned(),
        aliases: cmd.get_visible_aliases().map(ToString::to_string).collect(),
        help: cmd.get_about().map(ToString::to_string).unwrap_or_default(),
        args,
        children,
    }
}

fn arg_line(a: &ArgInfo) -> String {
    let mut s = a.name.clone();
    if let Some(v) = &a.value {
        let _ = write!(s, "  {v}");
    }
    if !a.choices.is_empty() {
        let _ = write!(s, " [{}]", a.choices.join("|"));
    }
    if let Some(d) = &a.default {
        let _ = write!(s, "  = {d}");
    }
    if !a.help.is_empty() {
        let _ = write!(s, "  — {}", a.help);
    }
    if a.global {
        s.push_str("  (global)");
    }
    s
}

/// Box-drawing tree, one line per command or argument.
#[must_use]
pub fn render(node: &Node) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "{}", node.name);
    render_children(node, "", &mut out);
    out
}

fn render_children(node: &Node, prefix: &str, out: &mut String) {
    let mut lines: Vec<(String, Option<&Node>)> =
        node.args.iter().map(|a| (arg_line(a), None)).collect();
    for c in &node.children {
        let mut label = c.name.clone();
        if !c.aliases.is_empty() {
            let _ = write!(label, " ({})", c.aliases.join(", "));
        }
        if !c.help.is_empty() {
            let _ = write!(label, "  {}", c.help);
        }
        lines.push((label, Some(c)));
    }
    let n = lines.len();
    for (i, (label, child)) in lines.into_iter().enumerate() {
        let last = i + 1 == n;
        let _ = writeln!(out, "{prefix}{} {label}", if last { "└──" } else { "├──" });
        if let Some(c) = child {
            let next = format!("{prefix}{}", if last { "    " } else { "│   " });
            render_children(c, &next, out);
        }
    }
}
