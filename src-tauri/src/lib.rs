mod config;
mod git;
mod osc;
mod pty;
mod recipes;
mod runner;
mod shell;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(pty::PtyState::default())
        .invoke_handler(tauri::generate_handler![
            pty::pty_spawn,
            pty::pty_write,
            pty::pty_resize,
            recipes::recipes_list,
            recipes::recipe_plan,
            recipes::recipe_run,
            recipes::git_branches,
            runner::recipe_cancel,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Branchdeck")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                app.state::<pty::PtyState>().shutdown();
            }
        });
}
