import {prepareBrowserImportRequest,approveBrowserImportRequest,cancelBrowserImportRequest,
  claimBrowserImportSourceRequest,authorizeBrowserImportSourceRequest}
  from '../../packages/kernel-client/src/browser-import-requests.ts';

const requests = {
  PrepareBrowserImport:[prepareBrowserImportRequest,'prepared'],
  ApproveBrowserImport:[approveBrowserImportRequest,'approved'],
  ClaimBrowserImportSource:[claimBrowserImportSourceRequest,'source_claimed'],
  AuthorizeBrowserImportSource:[authorizeBrowserImportSourceRequest,'source_authorized'],
};

export function snapshotImportRequest(value) {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error('invalid import metadata');
  const keys = Object.keys(value ?? {});
  if (keys.length !== 1) throw new Error('invalid import metadata');
  const kind = keys[0];
  const body = value[kind];
  if (!body || typeof body !== 'object' || Array.isArray(body)) throw new Error('invalid import metadata');
  if (kind === 'CancelBrowserImport') {
    const requestId = identifier(body.request_id);
    return {request:cancelBrowserImportRequest(text(body.session_id),text(body.attachment_id),requestId),
      status:'cancelled',requestId};
  }
  if (!Object.hasOwn(requests,kind)) throw new Error('invalid import metadata');
  const requestId = kind === 'PrepareBrowserImport' ? null : identifier(body.request_id);
  const source = body.selection;
  if (!source || typeof source !== 'object' || Array.isArray(source)) throw new Error('invalid import metadata');
  const selection = {};
  for (const key of ['session_id','attachment_id','environment_id','tab_id','source_store_id']) {
    selection[key] = text(source[key]);
  }
  for (const key of ['runtime_generation','document_revision']) {
    selection[key] = source[key];
    if (!Number.isSafeInteger(selection[key]) || selection[key] < 0) throw new Error('invalid import metadata');
  }
  selection.overwrite = source.overwrite;
  if (typeof selection.overwrite !== 'boolean') throw new Error('invalid import metadata');
  for (const key of ['domains','partition_sites']) {
    const items = source[key];
    if (!Array.isArray(items) || items.length > 32) throw new Error('invalid import metadata');
    const length = items.length;
    selection[key] = [];
    for (let index = 0; index < length; index++) selection[key].push(text(items[index],2048));
  }
  if (selection.domains.length === 0) throw new Error('invalid import metadata');
  const [builder,status] = requests[kind];
  return {request:kind === 'PrepareBrowserImport' ? builder(selection) : builder(requestId,selection),
    status,requestId};
}

function identifier(value) {
  if (typeof value !== 'string' || !/^[a-fA-F0-9]{32}$/.test(value)) throw new Error('invalid import metadata');
  return value;
}

function text(value,max = 512) {
  if (typeof value !== 'string' || !value || value.length > max || /[\x00-\x1f\x7f]/.test(value)) {
    throw new Error('invalid import metadata');
  }
  return value;
}
