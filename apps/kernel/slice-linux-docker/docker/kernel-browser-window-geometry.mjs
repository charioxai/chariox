// Native capture fitting and viewer resize mutate the same Chrome window.
const windows=new WeakMap();
export function withWindowGeometry(connection,operation){
 const run=(windows.get(connection)??Promise.resolve()).catch(()=>{}).then(operation);
 windows.set(connection,run);
 return run;
}
