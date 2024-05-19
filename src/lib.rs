pub mod comic;

use std::{
    cmp::Reverse,
    io::{BufRead, BufReader},
    process::{Command, Stdio},
};

use serde::Deserialize;

pub fn safe_path(value: &str) -> String {
    value
        .chars()
        .filter(|&c| c.is_alphanumeric() || c == ' ')
        .collect()
}

pub fn displays() -> anyhow::Result<Vec<String>> {
    #[derive(Debug, Deserialize)]
    struct Output {
        name: String,
        rect: Dimensions,
    }
    #[derive(Debug, Deserialize)]
    struct Dimensions {
        width: u32,
    }
    if let Ok(mut child) = Command::new("swaymsg")
        .args(["-t", "get_outputs"])
        .stdout(Stdio::piped())
        .spawn()
    {
        let mut outputs: Vec<Output> = serde_json::from_reader(child.stdout.take().unwrap())?;
        outputs.sort_by_key(|o| Reverse(o.rect.width));
        Ok(outputs.into_iter().map(|o| o.name).collect())
    } else {
        let mut child = Command::new("xrandr").stdout(Stdio::piped()).spawn()?;
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let outputs = stdout
            .lines()
            .filter_map(|line| {
                let line = line.ok()?;
                if line.contains(" connected ") {
                    line.split(' ').next().map(str::to_owned)
                } else {
                    None
                }
            })
            .collect();
        Ok(outputs)
    }
}
