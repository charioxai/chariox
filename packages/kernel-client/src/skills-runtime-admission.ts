// MP-08 / MP-10 / MP-11: protocol support does not imply a runnable Skills service.
import { SKILLS_V2_RUNTIME_CAPABILITY } from "./ipc-skills-v2-requests.js";
export async function skillsRuntimeAvailable(send: (request: { RelayStatus: null }) => Promise<unknown>): Promise<boolean> {
    const response = await send({ RelayStatus: null }) as { RelayStatus?: { status?: { capabilities?: unknown } } } | null;
    const capabilities = response?.RelayStatus?.status?.capabilities;
    return Array.isArray(capabilities) && capabilities.every(value => typeof value === "string")
        && capabilities.includes(SKILLS_V2_RUNTIME_CAPABILITY);
}
export async function requireSkillsRuntime(send: (request: { RelayStatus: null }) => Promise<unknown>): Promise<void> {
    if (!await skillsRuntimeAvailable(send))
        throw new Error("Skills v2 runtime is unavailable on this kernel. Upgrade the kernel and reselect any Skills chips.");
}
