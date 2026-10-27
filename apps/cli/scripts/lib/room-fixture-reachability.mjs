// Self-contained so the same bounded probe runs inside the owned test slice.
export async function probeRoomFixture(url, marker) {
  try {
    const response = await fetch(url, { signal: AbortSignal.timeout(5_000) })
    const body = await response.text()
    return { reachable: response.ok, status: response.status, markerPresent: body.includes(marker) }
  } catch (error) {
    return {
      reachable: false,
      errorName: error?.name ?? "Error",
      errorCode: typeof error?.cause?.code === "string" ? error.cause.code : null,
    }
  }
}

export function roomFixtureProbeCommand(url, marker) {
  return [
    "node", "--input-type=module", "-e",
    `console.log(JSON.stringify(await (${probeRoomFixture.toString()})(process.argv[1], process.argv[2])))`,
    url, marker,
  ]
}
