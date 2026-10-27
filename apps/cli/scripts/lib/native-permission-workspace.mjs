// MP-08/MP-10: owned native permission fixture uses the provider workspace.
import path from 'node:path';
export function permissionFixtureTarget(filePath, sliceRef) {
 return sliceRef ? path.posix.join('outputs',path.basename(filePath)) : filePath;
}
