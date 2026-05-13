/*
 * Copyright (c) Radical HQ Limited
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 */

#![warn(clippy::pedantic)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::too_many_lines,
    clippy::struct_excessive_bools,
    clippy::fn_params_excessive_bools,
    clippy::module_name_repetitions
)]

//! A command-line tool for submitting and updating GitHub Pull Requests from
//! local Git commits that may be amended and rebased. Pull Requests can be
//! stacked to allow for a series of code reviews of interdependent code.

use clap::{Parser, Subcommand};
use color_eyre::eyre::{Error, Result, eyre};
use log::debug;
use spr::commands;
use spr::error::SprError;

#[derive(Parser, Debug)]
#[clap(
    name = "spr",
    version,
    about = "Submit pull requests for individual, amendable, rebaseable commits to GitHub"
)]
pub struct Cli {
    /// Change to DIR before performing any operations
    #[clap(long, value_name = "DIR")]
    cd: Option<String>,

    /// GitHub personal access token (if not given taken from git config
    /// spr.githubAuthToken)
    #[clap(long)]
    github_auth_token: Option<String>,

    /// GitHub repository ('org/name', if not given taken from config
    /// spr.githubRepository)
    #[clap(long)]
    github_repository: Option<String>,

    /// The default branch into which pull requests are merged (if not given
    /// taken from git config spr.githubDefaultBranch, falling back to
    /// spr.githubMasterBranch, defaulting to 'master')
    #[clap(long)]
    github_default_branch: Option<String>,

    /// prefix to be used for branches created for pull requests (if not given
    /// taken from git config spr.branchPrefix, defaulting to
    /// 'spr/<`GITHUB_USERNAME`>/')
    #[clap(long)]
    branch_prefix: Option<String>,

    /// Never prompt for input; fail instead of waiting for user interaction.
    /// Also enabled by setting `SPR_NON_INTERACTIVE=1`.
    #[clap(long, global = true)]
    non_interactive: bool,

    /// Suppress decorative output; only print essential information (URLs, PR numbers).
    /// Also enabled by setting `SPR_QUIET=1`.
    #[clap(long, short = 'q', global = true)]
    quiet: bool,

    /// Preview actions without performing them.
    /// Also enabled by setting `SPR_DRY_RUN=1`.
    #[clap(long, global = true)]
    dry_run: bool,

    /// Show detailed progress for each action.
    /// Also enabled by setting `SPR_VERBOSE=1`.
    #[clap(long, global = true)]
    verbose: bool,

    #[clap(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Interactive assistant for configuring spr in a local GitHub-backed Git
    /// repository
    Init,

    /// Create a new or update an existing Pull Request on GitHub from the
    /// current HEAD commit
    Diff(commands::diff::DiffOptions),

    /// Reformat commit message
    Format(commands::format::FormatOptions),

    /// Land a reviewed Pull Request
    Land(commands::land::LandOptions),

    /// Update local commit message with content on GitHub
    Amend(commands::amend::AmendOptions),

    /// List open Pull Requests on GitHub and their review decision
    List,

    /// Create a new branch with the contents of an existing Pull Request
    Patch(commands::patch::PatchOptions),

    /// Close a Pull request
    Close(commands::close::CloseOptions),

    /// Show status of the Pull Request for HEAD commit
    Status(commands::status::StatusOptions),

    /// Validate commit message and check for cherry-pick conflicts (fetches from remote)
    Check(commands::check::CheckOptions),

    /// Rebase local branch onto upstream default branch and optionally update PRs
    Sync(commands::sync::SyncOptions),
}

pub async fn spr() -> Result<()> {
    let cli = Cli::parse();
    debug!("Started with command line: {cli:?}");

    let quiet = cli.quiet
        || std::env::var("SPR_QUIET")
            .ok()
            .is_some_and(|v| v == "1" || v == "true");
    spr::output::set_quiet(quiet);

    let verbose = cli.verbose
        || std::env::var("SPR_VERBOSE")
            .ok()
            .is_some_and(|v| v == "1" || v == "true");

    let dry_run = cli.dry_run
        || std::env::var("SPR_DRY_RUN")
            .ok()
            .is_some_and(|v| v == "1" || v == "true");

    if quiet && verbose {
        color_eyre::eyre::bail!("--quiet and --verbose are mutually exclusive");
    }

    if let Some(path) = &cli.cd
        && let Err(err) = std::env::set_current_dir(path)
    {
        eprintln!("Could not change directory to {:?}", &path);
        return Err(err.into());
    }

    if let Commands::Init = cli.command {
        return commands::init::init().await;
    }

    let repo = git2::Repository::discover(std::env::current_dir()?)?;

    let git_config = repo.config()?;

    let github_repository = match cli.github_repository {
        Some(v) => v,
        None => git_config.get_string("spr.githubRepository").map_err(|_| {
            eyre!(SprError::Auth(
                "spr.githubRepository not configured. Run 'spr init' to set up this repository."
                    .to_string(),
            ))
        })?,
    };

    let github_default_branch = match cli.github_default_branch {
        Some(v) => Ok::<String, git2::Error>(v),
        None => git_config
            .get_string("spr.githubDefaultBranch")
            .or_else(|_| git_config.get_string("spr.githubMasterBranch"))
            .or_else(|_| Ok("master".to_string())),
    }?;

    let branch_prefix = match cli.branch_prefix {
        Some(v) => v,
        None => git_config.get_string("spr.branchPrefix").map_err(|_| {
            eyre!(SprError::Auth(
                "spr.branchPrefix not configured. Run 'spr init' to set up this repository."
                    .to_string(),
            ))
        })?,
    };

    let (github_owner, github_repo) = {
        let captures = lazy_regex::regex!(r#"^([\w\-\.]+)/([\w\-\.]+)$"#)
            .captures(&github_repository)
            .ok_or_else(|| {
                eyre!(
                    "GitHub repository must be given as 'OWNER/REPO', but given value was '{}'",
                    &github_repository,
                )
            })?;
        (
            captures.get(1).unwrap().as_str().to_string(),
            captures.get(2).unwrap().as_str().to_string(),
        )
    };

    let require_approval = git_config
        .get_bool("spr.requireApproval")
        .ok()
        .unwrap_or(false);
    let require_test_plan = git_config
        .get_bool("spr.requireTestPlan")
        .ok()
        .unwrap_or(true);
    let create_draft_prs = git_config
        .get_bool("spr.createDraftPRs")
        .ok()
        .unwrap_or(false);

    let github_auth_token = match cli.github_auth_token {
        Some(v) => v,
        None => spr::token::find_token("github.com")
            .or_else(|| git_config.get_string("spr.githubAuthToken").ok())
            .ok_or_else(|| eyre!(SprError::Auth(
                "No GitHub auth token found. Set GITHUB_TOKEN, run 'gh auth login', \
                 or run 'spr init' to configure one.".into()
            )))?,
    };

    let non_interactive = cli.non_interactive
        || std::env::var("SPR_NON_INTERACTIVE")
            .ok()
            .is_some_and(|v| v == "1" || v == "true");

    let default_reviewers = git_config
        .get_string("spr.defaultReviewers")
        .ok()
        .map(|s| {
            s.split(',')
                .map(|r| r.trim().to_string())
                .filter(|r| !r.is_empty())
                .collect()
        })
        .unwrap_or_default();

    let merge_method = git_config
        .get_string("spr.mergeMethod")
        .ok()
        .map(|s| spr::config::MergeMethod::parse(&s))
        .unwrap_or_default();

    let config = spr::config::Config::new(
        github_owner,
        github_repo,
        &github_default_branch,
        branch_prefix,
        github_auth_token.clone(),
        require_approval,
        require_test_plan,
        create_draft_prs,
        non_interactive,
        default_reviewers,
        merge_method,
    );
    debug!("config: {config:?}");

    let git = spr::git::Git::new(repo);

    octocrab::initialise(
        octocrab::Octocrab::builder()
            .personal_token(github_auth_token.clone())
            .build()?,
    );

    let gh = spr::github::GitHub::new(
        config.clone(),
        git.clone(),
        github_auth_token,
    );
    let forge: Box<dyn spr::forge::ForgeApi> = if dry_run {
        Box::new(spr::forge::DryRunForge::new(Box::new(gh), verbose))
    } else if verbose {
        Box::new(spr::forge::VerboseForge::new(gh))
    } else {
        Box::new(gh)
    };

    match cli.command {
        Commands::Diff(opts) => {
            commands::diff::diff(opts, &git, &*forge, &config).await?;
        }
        Commands::Land(opts) => {
            commands::land::land(opts, &git, &*forge, &config).await?;
        }
        Commands::Amend(opts) => {
            commands::amend::amend(opts, &git, &*forge, &config).await?;
        }
        Commands::List => commands::list::list(&config).await?,
        Commands::Patch(opts) => {
            commands::patch::patch(opts, &git, &*forge, &config).await?;
        }
        Commands::Close(opts) => {
            commands::close::close(opts, &git, &*forge, &config).await?;
        }
        Commands::Status(opts) => {
            commands::status::status(opts, &git, &*forge, &config).await?;
        }
        Commands::Check(opts) => {
            commands::check::check(opts, &git, &*forge, &config).await?;
        }
        Commands::Sync(opts) => {
            commands::sync::sync(opts, &git, &*forge, &config).await?;
        }
        Commands::Format(opts) => {
            commands::format::format(opts, &git, &*forge, &config).await?;
        }
        // Init is handled above and returns early.
        Commands::Init => (),
    }

    Ok::<_, Error>(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    env_logger::init();

    let result = tokio::task::LocalSet::new().run_until(spr()).await;
    match result {
        Ok(()) => std::process::exit(0),
        Err(err) => {
            eprintln!("error: {err:#}");

            let exit_code = err
                .downcast_ref::<SprError>()
                .map_or(1, SprError::exit_code);

            std::process::exit(exit_code);
        }
    }
}
