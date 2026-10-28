#[macro_use]
pub mod i18n;
pub mod automation;
pub mod bandwidth;
pub mod bilibili;
pub mod capture;
pub mod categories;
pub mod control;
pub mod dash;
pub mod diagnostics;
pub mod engine;
pub mod ftp;
pub mod grabber_projects;
pub mod grabber_schedule;
pub mod hls;
pub mod mcp;
pub mod mega;
pub mod model;
pub mod native;
pub mod notifications;
pub mod offline_website;
pub mod paths;
pub mod preferences;
pub mod proxy;
pub mod quota;
pub mod retry;
pub mod scheduler;
pub mod script_automation;
pub mod store;
pub mod synchronization;
pub mod tiktok;
pub mod twitch;
pub mod updates;
pub mod website_grabber;
pub mod wetransfer;

pub mod queue;
pub mod queue_schedule;
mod queue_start;
mod queue_store;

mod download_file;

pub mod browser_batch;
pub mod browser_recovery;
pub mod browser_setup;
pub mod browser_video;

pub mod link_fallback;
pub mod link_recovery;

pub mod auth;
mod credential_vault;

pub mod antivirus;
pub mod integrity;
pub mod list_transfer;

pub mod completion;
pub mod connections;
mod server_filename;
mod status_requests;
pub mod table_layout;
pub mod transfer_activity;

pub mod browser_login;
mod protected_data;

mod child_process;
mod network_scope;
pub mod youtube;
pub mod zip_preview;

#[cfg(windows)]
mod windows_http;
