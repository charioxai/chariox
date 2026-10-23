// MP-08 / MP-10 / MP-11: admission guard only; resolution remains kernel-owned.
import assert from "node:assert/strict"
import test from "node:test"
import { hasExplicitSkillMarker } from "./skill-marker.js"

test("typed skill markers require the v2 resolver", () => {
  for (const prompt of ["use $review", "($review)", "use $Review_1.", "it's $review", "[$review]"]) {
    assert.equal(hasExplicitSkillMarker(prompt), true, prompt)
  }
})
test("ordinary source, links, shell variables and currency retain text admission", () => {
  for (const prompt of ["echo $HOME", "$123", "\\$review", "`$review`", "``$review``",
    "'$review'", '\"$review\"', "```sh\n$review\n```", "~~~\n$review\n~~~", "    $review",
    "[$review](https://example.org)", "file/$review", "$review/path"]) {
    assert.equal(hasExplicitSkillMarker(prompt), false, prompt)
  }
})
