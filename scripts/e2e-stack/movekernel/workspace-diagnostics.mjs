// MP-08 / MP-09 / MP-10 / MP-11: retain probe stages without private log content.
import {readFile, readdir} from 'node:fs/promises';

export function checkpointDiagnostic(row) {
    if (row?.component !== 'provider.checkpoint' || row.message !== 'Native checkpoint probe failed') return;
    return {
        timestamp_ms: Number.isSafeInteger(row.timestamp_ms) ? row.timestamp_ms : undefined,
        providerRunId: /^provider-run-\d+$/.test(row.provider_run_id) ? row.provider_run_id : undefined,
        stage: ['connect', 'thread/resume'].includes(row.stage) ? row.stage : undefined,
        operation: ['connect', 'initialize', 'thread/resume', 'native_checkpoint'].includes(row.operation) ? row.operation : undefined,
    };
}

export async function checkpointDiagnostics(home) {
    const directory = home + '/logs', result = [];
    for (const name of await readdir(directory).catch(error => {
        if (error.code === 'ENOENT') return [];
        throw error;
    })) {
        if (!name.endsWith('.ndjson')) continue;
        for (const line of (await readFile(directory + '/' + name, 'utf8')).split('\n')) {
            let row;
            try { row = JSON.parse(line); } catch { continue; }
            const diagnostic = checkpointDiagnostic(row);
            if (diagnostic) result.push(diagnostic);
        }
    }
    return result;
}
