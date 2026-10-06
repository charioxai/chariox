# MP-08/MP-10/MP-11 — phase20 performance IN PROGRESS

2026-10-06: required base7cf42bc9014994022cd47708eaaff29a729f1cb7; branch md/display-perf. Local447/relay90 unchanged. Read phase19 report and lane history. Measure capture/convert/encode/packetize/client stages before runtime changes, fix dominant stage first, preserve exact settle and fail-closed codec masking. Targets DPR2>=30fps/<50ms input;1080p<=1pipeline core/<50ms input and beat supplied Selkies baseline. Final210-cycle zero-violation protection gate required. Local commits only [skip ci]; no push/CI/deploy.

## MP-08/MP-10 Coordinator asks

Real live matrix remains coordinator-run per phase20 assignment: actual Cloud app+flags, hostedwss~8Mbps/RTT>=60ms, public-site list DPR1/2 and multi-hour stability. Lane forbidden to contact hosted relay/Apps. Deliver exact kits/commands and per-condition local limitations. No protocol allocation needed yet.

## MP-11 Review inbox

Absolute lane review inbox absent at start; recheck after every milestone/commit. No semantic approval inferred. Own disposable runtime identities only; credentials/shared services/caches/foreign resources excluded from cleanup.

## MP-08/MP-10/MP-11 phase20 source checkpoint

Before changes: protected DPR2 stage diagnostic6.88fps, click/typeP9574.0/68.7ms, pipeline2.12cores. Native mask copy/hash14.43msP50; full stripe encode~16ms plus raw IPC; queued event batching33ms remains a separate input delay. Scheduling CRC over masked bytes raises motion15.53fps; immutable file handoff preview24.01fps. Two workers regresses20.87fps and is rejected (explicit experiment only, default remains1). None passes full performance acceptance. Small masked damage now retains exact readback bounds only with identical preceding trusted masks; transitions stay full repair. Codec guards unchanged. Display-enabled kernel event delay0 retains ordinary bounded event lane/control priority; wire/protocol447/90 unchanged. Await exact rebuilt runtime and final210-cycle gate.
