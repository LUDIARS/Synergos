#include "synergos/custody/signature.hpp"
#include <stdexcept>
#include <string>

using namespace synergos::custody;
namespace {
void require(bool value) { if (!value) throw std::runtime_error("Custody signature contract failed"); }
void rejects(const std::string& value) {
    try { canonical(value); } catch (const std::invalid_argument&) { return; }
    throw std::runtime_error("Invalid canonical signature input accepted");
}
}
int main() {
    require(object_key("abc") == "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    require(object_key("") == "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    // RFC 4231 test cases 1 and 6: binary key and key exceeding SHA256 block size.
    require(sign(std::string(20, '\x0b'), "Hi There") ==
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7");
    require(sign(std::string(131, '\xaa'), "Test Using Larger Than Block-Size Key - Hash Key First") ==
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54");
    require(sign("", "") == "b613679a0814d9ec772f95d778c35fc5ff1697c493715653c6c712144292c5ad");
    const auto receipt = canonical(R"({"operationId":"11111111-1111-4111-8111-111111111111","opaqueObjectId":"uploads/asset/object","verifiedDigests":[{"algorithm":"sha256","value":"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"}],"byteLength":3,"durability":"durable","locations":["22222222-2222-4222-8222-222222222222"],"expiresAt":"2030-01-01T00:00:00.000Z"})");
    const auto secret = "dummy-receipt-key-for-cu-synergos-tests";
    const auto signature = sign(secret, receipt);
    require(signature == "cb241fe664ef6b04b86724ce98a6a6d410ff4b6e0f964ed88d68fb73210863cf");
    require(verify(secret, receipt, signature));
    require(!verify(secret, "tampered", signature));
    require(!verify("wrong key", receipt, signature));
    for (std::size_t i = 0; i < signature.size(); ++i) {
        auto changed = signature;
        changed[i] = changed[i] == '0' ? '1' : '0';
        require(!verify(secret, receipt, changed));
    }
    require(!verify(secret, receipt, std::string(64, 'A')));
    require(!verify(secret, receipt, signature.substr(1)));
    require(canonical(R"({"z":[{"b":2,"a":"素材"}],"a":true})") ==
            R"({"a":true,"z":[{"a":"素材","b":2}]})");
    require(capability_message("GET", "abc", 42, "試作.bin") ==
            R"({"expires":42,"key":"abc","method":"GET","name":"試作.bin"})");
    rejects("{\"value\":1.25}");
    require(canonical(R"({"escape":"\u001B","literal":"\\u001B","zero":"\u0000"})") ==
            R"({"escape":"\u001b","literal":"\\u001B","zero":"\u0000"})");
    require(canonical(R"([18446744073709551615,-9223372036854775808,true,null])") ==
            "[18446744073709551615,-9223372036854775808,true,null]");
    rejects(R"({"duplicate":1,"duplicate":2})");
    rejects(R"([1,])");
    rejects(std::string("\"") + static_cast<char>(0xff) + "\"");
    rejects(std::string(129, '[') + "0" + std::string(129, ']'));
    rejects(std::string(1024 * 1024 + 1, ' '));
}
