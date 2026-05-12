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

pub mod commands;
pub mod config;
pub mod forge;
pub mod git;
pub mod git_remote;
pub mod github;
pub mod message;
pub mod output;
pub mod token;
pub mod utils;
