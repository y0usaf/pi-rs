-- Exerciser for the reviewed crypto/buffer mechanisms: pi.crypto.* and
-- pi.buffer.*. Translations from Node/Bun primitives the pinned dogfood
-- suite relies on:
--   node:crypto#createHash   → pi.crypto.sha1/sha256/md5
--   node:crypto#randomUUID   → pi.crypto.random_uuid
--   Bun.hash.xxHash32        → pi.crypto.xxhash32
--   Buffer.base64*           → pi.buffer.base64_encode/decode
--   Buffer.byteLength        → pi.buffer.byte_length
local pi = ...

pi.register_command("crypto-demo", {
  description = "Exercise the reviewed crypto/buffer mechanisms",
  handler = function()
    local sha1 = pi.crypto.sha1("hello")
    local sha256 = pi.crypto.sha256("hello")
    local md5 = pi.crypto.md5("hello")
    local xx32 = pi.crypto.xxhash32("hello", 0)

    local uuid = pi.crypto.random_uuid()
    local uuid_shape = uuid:match("^%x%x%x%x%x%x%x%x%-%x%x%x%x%-4%x%x%x%-[89ab]%x%x%x%-%x%x%x%x%x%x%x%x%x%x%x%x$") ~= nil
    local unique = uuid ~= pi.crypto.random_uuid()

    local b64 = pi.buffer.base64_encode("hello")
    local decoded = pi.buffer.base64_decode(b64)
    local byte_len = pi.buffer.byte_length("hello")

    return {
      sha1 = sha1, sha256 = sha256, md5 = md5,
      xxhash32 = xx32,
      uuid_shape = uuid_shape, unique = unique,
      base64 = b64, decoded = decoded, byte_len = byte_len,
    }
  end,
})
