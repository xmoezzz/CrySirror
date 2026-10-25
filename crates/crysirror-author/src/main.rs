// SPDX-License-Identifier: MPL-2.0
use anyhow::{ensure, Context, Result};
use std::{
    io::{self, Write},
    path::PathBuf,
};

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let input =
        PathBuf::from(args.next().context(
            "usage: crysirror-author <project.ts|project.js> (validated JSON to stdout)",
        )?);
    ensure!(
        args.next().is_none(),
        "usage: crysirror-author <project.ts|project.js>"
    );
    let project = crysirror_author::build_file(&input)?;
    // Finish validation and encoding before writing any stdout bytes.
    let mut json = serde_json::to_vec_pretty(&project)?;
    json.push(b'\n');
    ensure!(
        json.len() <= crysirror_runtime::model::MAX_PROJECT_BYTES,
        "encoded project exceeds 4 MiB"
    );
    io::stdout().lock().write_all(&json)?;
    Ok(())
}
