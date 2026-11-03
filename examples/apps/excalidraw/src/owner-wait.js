// The owner answers a file request in their own time. One call ends at the
// kernel's call deadline, but the kernel keeps the same prompt (up to ten
// minutes) and a repeated ask resumes it.
export const OWNER_ASKS = 20;

export async function untilOwnerAnswers(ask) {
  for (let attempt = 1; ; attempt += 1) {
    try { return await ask(); } catch (error) { if (error?.code !== 'DEADLINE_EXCEEDED' || attempt >= OWNER_ASKS) throw error; }
  }
}
