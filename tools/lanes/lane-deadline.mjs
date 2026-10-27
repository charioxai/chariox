#!/usr/bin/env node
// MP-08 / MP-09 / MP-10 / MP-11: foreground coordinator timer retains its grant ancestry.
import { main } from "./lanes.mjs";
await main("deadline");
