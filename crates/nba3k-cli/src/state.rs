use anyhow::{anyhow, Result};
use nba3k_core::GameMode;
use nba3k_store::Store;
use std::path::PathBuf;

pub struct AppState {
    pub save_path: Option<PathBuf>,
    pub force_god: bool,
    store: Option<Store>,
    pub should_quit: bool,
}

impl AppState {
    pub fn new(save_path: Option<PathBuf>, force_god: bool) -> Self {
        Self {
            save_path,
            force_god,
            store: None,
            should_quit: false,
        }
    }

    pub fn store(&mut self) -> Result<&mut Store> {
        if self.store.is_none() {
            self.open_from_save()?;
        }
        self.store
            .as_mut()
            .ok_or_else(|| anyhow!("no save loaded; pass --save <path> or run `load <path>`"))
    }

    pub fn open_path(&mut self, path: PathBuf) -> Result<()> {
        let s = Store::open(&path)?;
        self.store = Some(s);
        self.save_path = Some(path);
        Ok(())
    }

    fn open_from_save(&mut self) -> Result<()> {
        let path = self
            .save_path
            .clone()
            .ok_or_else(|| anyhow!("no --save path set"))?;
        let s = Store::open(&path)?;
        self.store = Some(s);
        Ok(())
    }

    pub fn effective_mode(&self, on_save: GameMode) -> GameMode {
        if self.force_god {
            GameMode::God
        } else {
            on_save
        }
    }
}
