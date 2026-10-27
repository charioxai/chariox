// MP-08/MP-10: host paths are inaccessible to the slice user.
import test from 'node:test';import assert from 'node:assert/strict';import {permissionFixtureTarget} from './native-permission-workspace.mjs';
test('slice target is relative to kernel-managed provider workspace',()=>assert.equal(permissionFixtureTarget('/root/owned/workspace/outputs/native-proof.txt','slice-owned'),'outputs/native-proof.txt'));
test('standard worker retains its authorized execution path',()=>assert.equal(permissionFixtureTarget('/root/owned/workspace/outputs/native-proof.txt',null),'/root/owned/workspace/outputs/native-proof.txt'));
