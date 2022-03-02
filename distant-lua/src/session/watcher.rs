use crate::runtime;
use distant_core::{data::Change as DistantChange, Watcher as DistantWatcher};
use mlua::{prelude::*, UserData, UserDataFields, UserDataMethods};
use std::{
    ops::{Deref, DerefMut},
    path::PathBuf,
    sync::Arc,
};
use tokio::sync::RwLock;

#[derive(Clone, Debug)]
pub struct Change(DistantChange);

impl From<DistantChange> for Change {
    fn from(watcher: DistantChange) -> Self {
        Self(watcher)
    }
}

impl Deref for Change {
    type Target = DistantChange;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Change {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl UserData for Change {
    fn add_fields<'lua, F: UserDataFields<'lua, Self>>(fields: &mut F) {
        fields.add_field_method_get("kind", |_, this| Ok(this.kind.to_string()));
        fields.add_field_method_get("paths", |_, this| {
            Ok(this
                .paths
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect::<Vec<String>>())
        });
    }
}

#[derive(Clone, Debug)]
pub struct Watcher {
    path: PathBuf,
    inner: Arc<RwLock<DistantWatcher>>,
}

impl From<DistantWatcher> for Watcher {
    fn from(watcher: DistantWatcher) -> Self {
        let path = watcher.path().to_path_buf();
        Self {
            path,
            inner: Arc::new(RwLock::new(watcher)),
        }
    }
}

impl UserData for Watcher {
    fn add_fields<'lua, F: UserDataFields<'lua, Self>>(fields: &mut F) {
        fields.add_field_method_get("path", |_, this| {
            Ok(this.path.to_string_lossy().to_string())
        });
    }

    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        methods.add_method_mut("next", |_lua, this, ()| next(this.clone()));
        methods.add_async_method("next_async", |_lua, this, ()| {
            runtime::spawn(next_async(this))
        });
        methods.add_method_mut("unwatch", |_lua, this, ()| unwatch(this.clone()));
        methods.add_async_method("unwatch_async", |_lua, this, ()| {
            runtime::spawn(unwatch_async(this))
        });
    }
}

fn next(watcher: Watcher) -> LuaResult<Option<Change>> {
    runtime::block_on(next_async(watcher))
}

async fn next_async(watcher: Watcher) -> LuaResult<Option<Change>> {
    Ok(watcher.inner.write().await.next().await.map(Change::from))
}

fn unwatch(watcher: Watcher) -> LuaResult<()> {
    runtime::block_on(unwatch_async(watcher))
}

async fn unwatch_async(watcher: Watcher) -> LuaResult<()> {
    watcher.inner.write().await.unwatch().await.to_lua_err()
}
