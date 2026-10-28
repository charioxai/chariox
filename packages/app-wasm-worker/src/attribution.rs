use crate::scope::Scope;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Instant,
};
use wasmtime::component::{TaskGroupHook, TaskGroupId};

#[derive(Default)]
struct State {
    starting: Option<Scope>,
    groups: HashMap<TaskGroupId, Scope>,
    current: Option<Scope>,
    last: Option<Scope>,
    slice: Option<Instant>,
}
#[derive(Clone, Default)]
pub struct Attribution(Arc<Mutex<State>>);
impl Attribution {
    pub fn starting(&self, scope: Scope) {
        self.0.lock().unwrap().starting = Some(scope);
    }
    pub fn current(&self) -> Option<Scope> {
        self.0.lock().unwrap().current.clone()
    }
    pub fn culprit(&self) -> Option<String> {
        self.0
            .lock()
            .unwrap()
            .last
            .as_ref()
            .map(|scope| scope.id().into())
    }
    pub fn slice_started(&self) -> Option<Instant> {
        self.0.lock().unwrap().slice
    }
}
impl TaskGroupHook for Attribution {
    fn handle_start(&mut self, id: TaskGroupId) -> wasmtime::Result<()> {
        let mut state = self.0.lock().unwrap();
        if let Some(scope) = state.starting.take() {
            if state.groups.len() >= 128 {
                state.last = Some(scope);
                return Err(crate::limits::Stop("APP_RESOURCE_LIMIT").into());
            }
            state.groups.insert(id, scope);
        }
        Ok(())
    }
    fn handle_enter(&mut self, id: TaskGroupId) -> wasmtime::Result<()> {
        let mut state = self.0.lock().unwrap();
        state.current = state.groups.get(&id).cloned();
        state.last = state.current.clone();
        state.slice = Some(Instant::now());
        Ok(())
    }
    fn handle_exit(&mut self, _id: TaskGroupId) -> wasmtime::Result<()> {
        let mut state = self.0.lock().unwrap();
        state.current = None;
        state.slice = None;
        Ok(())
    }
    fn handle_finish(&mut self, id: TaskGroupId) -> wasmtime::Result<()> {
        self.0.lock().unwrap().groups.remove(&id);
        Ok(())
    }
}
