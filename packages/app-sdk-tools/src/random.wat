;; Toolchain-only compatibility: derive legacy insecure RNG from the admitted
;; random interface instead of adding another host import or freezing a seed.
(component
  (type $random (instance
    (export "get-random-bytes" (func (param "len" u64) (result (list u8))))
    (export "get-random-u64" (func (result u64)))))
  (import "wasi:random/random@0.2.0" (instance $random (type $random)))
  (alias export $random "get-random-u64" (func $random-u64))
  (alias export $random "get-random-bytes" (func $random-bytes))
  (core func $random-u64 (canon lower (func $random-u64)))
  (core module $seed
    (import "random" "u64" (func $random (result i64)))
    (memory (export "memory") 1)
    (func (export "seed") (result i32)
      i32.const 0 call $random i64.store
      i32.const 8 call $random i64.store
      i32.const 0))
  (core instance $random-core (export "u64" (func $random-u64)))
  (core instance $seed (instantiate $seed (with "random" (instance $random-core))))
  (alias core export $seed "memory" (core memory $memory))
  (alias core export $seed "seed" (core func $seed))
  (func $seed (result (tuple u64 u64)) (canon lift (core func $seed) (memory $memory)))
  (instance $insecure-seed (export "insecure-seed" (func $seed)))
  (instance $insecure
    (export "get-insecure-random-u64" (func $random-u64))
    (export "get-insecure-random-bytes" (func $random-bytes)))
  (export "wasi:random/insecure-seed@0.2.0" (instance $insecure-seed))
  (export "wasi:random/insecure@0.2.0" (instance $insecure)))
