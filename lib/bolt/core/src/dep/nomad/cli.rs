use anyhow::*;
use std::process::Command;

use crate::{context::ProjectContext, utils::command_helper::CommandHelper};

pub struct LogsOpts {
	pub follow: bool,
	pub stream: LogStream,
}

pub enum LogStream {
	StdOut,
	StdErr,
}

pub async fn logs(ctx: &ProjectContext, service_name: &str, opts: &LogsOpts) -> Result<()> {
	let primary_region = ctx.primary_region();

	let mut cmd = Command::new("nomad");
	cmd.arg("alloc")
		.arg("logs")
		.arg("-job")
		.arg(format!("rivet-{}:{}", service_name, primary_region));
	if opts.follow {
		cmd.arg("-f");
	}
	if matches!(opts.stream, LogStream::StdErr) {
		cmd.arg("-stderr");
	}

	cmd.exec().await
}