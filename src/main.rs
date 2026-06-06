mod app;
mod audio;
mod game;
mod input;
mod physics;
mod renderer;
mod table;
mod ui;

use anyhow::Result;

fn main() -> Result<()> {
    let mut app = app::App::new()?;
    app.run()
}
