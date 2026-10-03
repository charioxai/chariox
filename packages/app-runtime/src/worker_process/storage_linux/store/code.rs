//! Code-view setup belongs to the existing authenticated storage lease and its
//! durable recovery journal. There is no second privileged service or launcher.
use super::super::{
    code_model::{self, Kind, Record, View},
    code_mounts,
    code_sources::Sources,
};
use super::*;

impl Store {
    pub fn attach_code(
        &mut self,
        uid: u32,
        lease: &str,
        package: &str,
        runtime: &str,
        revision: u64,
    ) -> Result<code_model::Grant> {
        model::hex(lease, 32)?;
        code_model::validate_pins(package, runtime, revision)?;
        let owner = self.owners.get(&uid).ok_or(Error::Identity)?;
        let live = self.live.get_mut(lease).ok_or(Error::Identity)?;
        if live.journal.uid != uid {
            return Err(Error::Identity);
        }
        live.bound.require_empty()?;
        if let Some(record) = &live.journal.code {
            if record.package_digest != package
                || record.runtime_digest != runtime
                || record.runtime_revision != revision
            {
                return Err(Error::Identity);
            }
            return grant(&live.directory, record);
        }
        let sources = Sources::open(owner, package, runtime, revision)?;
        let mut views = Vec::new();
        for (index, kind) in [Kind::Package, Kind::Runtime].into_iter().enumerate() {
            let underlying = files::child(&live.directory, kind.name(), 0o700)?;
            if !underlying.entries(1)?.is_empty() {
                return Err(Error::Identity);
            }
            views.push(View {
                kind,
                source: files::identity(sources.file(index))?,
                underlying: files::identity(&underlying.0)?,
                mount_id: None,
                sealed: false,
            });
        }
        live.code_sources = Some(sources);
        live.journal.code = Some(Record {
            package_digest: package.into(),
            runtime_digest: runtime.into(),
            runtime_revision: revision,
            views: views.try_into().map_err(|_| Error::Identity)?,
        });
        files::save_journal(&live.directory, &live.journal)?;
        for index in 0..2 {
            let source = live
                .code_sources
                .as_ref()
                .ok_or(Error::Identity)?
                .file(index);
            let view = &mut live.journal.code.as_mut().ok_or(Error::Identity)?.views[index];
            code_mounts::mount(&live.directory, &live.path, view, source)?;
            files::save_journal(&live.directory, &live.journal)?;
        }
        grant(
            &live.directory,
            live.journal.code.as_ref().ok_or(Error::Identity)?,
        )
    }
}

fn grant(parent: &Dir, record: &Record) -> Result<code_model::Grant> {
    record.validate()?;
    for view in &record.views {
        if !view.sealed {
            return Err(Error::RecoveryRequired);
        }
        let mounted = parent.child(OsStr::new(view.kind.name()))?;
        if Some(code_mounts::observe(&mounted, view.kind, &view.source)?) != view.mount_id {
            return Err(Error::Identity);
        }
    }
    Ok(code_model::Grant {
        package_digest: record.package_digest.clone(),
        runtime_digest: record.runtime_digest.clone(),
        runtime_revision: record.runtime_revision,
        package_root: record.views[0].source.clone(),
        runtime_root: record.views[1].source.clone(),
    })
}

pub(super) fn cleanup(parent: &Dir, path: &Path, journal: &mut Journal) -> Result<()> {
    let Some(record) = &journal.code else {
        return Ok(());
    };
    record.validate()?;
    for index in 0..2 {
        let view = &journal.code.as_ref().ok_or(Error::Identity)?.views[index];
        code_mounts::unmount(parent, path, view)?;
        let view = &mut journal.code.as_mut().ok_or(Error::Identity)?.views[index];
        view.mount_id = None;
        view.sealed = false;
        files::save_journal(parent, journal)?;
    }
    journal.code = None;
    files::save_journal(parent, journal)
}
