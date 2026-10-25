// @implements SPEC-SY-native-metadata SPEC-SY-decode-bounds
// @decoder core.manifest
// @decoder core.version_state
#include "synergos/core/manifest_codec.hpp"
#include <yyjson.h>
#include <cstdlib>
#include <limits>
#include <memory>
#include <set>
#include <stdexcept>

namespace synergos::core {
DecodeTooLarge::DecodeTooLarge(std::string decoder_id, std::size_t declared, std::size_t max)
    : std::length_error("decode.too_large " + decoder_id + ": " + std::to_string(declared) + " > " + std::to_string(max)),
      decoder_id_(std::move(decoder_id)), declared_(declared), max_(max) {}

const std::string& DecodeTooLarge::decoder_id() const noexcept { return decoder_id_; }
std::size_t DecodeTooLarge::declared() const noexcept { return declared_; }
std::size_t DecodeTooLarge::max() const noexcept { return max_; }

namespace {
using Document = std::unique_ptr<yyjson_doc, decltype(&yyjson_doc_free)>;
void bounded(const char* decoder_id, std::size_t declared, std::size_t max) {
    if (declared > max) throw DecodeTooLarge(decoder_id, declared, max);
}
std::string_view string_value(yyjson_val* value) {
    if (!yyjson_is_str(value)) throw std::invalid_argument("Expected JSON string");
    return {yyjson_get_str(value), yyjson_get_len(value)};
}
void object(yyjson_val* value) {
    if (!yyjson_is_obj(value)) throw std::invalid_argument("Expected JSON object");
    std::set<std::string_view> keys;
    auto iter = yyjson_obj_iter_with(value);
    while (auto* key = yyjson_obj_iter_next(&iter)) {
        if (!keys.insert(string_value(key)).second) throw std::invalid_argument("Duplicate JSON key");
    }
}
std::uint64_t unsigned_integer(yyjson_val* value) {
    if (!yyjson_is_uint(value)) throw std::invalid_argument("Expected unsigned integer");
    return yyjson_get_uint(value);
}
Document document(std::string_view text, std::string_view project_id, bool allow_empty_id) {
    if (project_id.empty()) throw std::invalid_argument("Project id is required");
    Document doc(yyjson_read(text.data(), text.size(), 0), yyjson_doc_free);
    if (!doc) throw std::invalid_argument("Invalid metadata JSON");
    auto* root = yyjson_doc_get_root(doc.get());
    object(root);
    if (auto* format = yyjson_obj_get(root, "format"); format && unsigned_integer(format) != 1) {
        throw std::invalid_argument("Unsupported metadata format");
    }
    const auto actual = string_value(yyjson_obj_get(root, "project_id"));
    if (actual != project_id && !(allow_empty_id && actual.empty())) throw std::invalid_argument("Wrong project");
    return doc;
}
// Own mutable allocations without a template JSON DOM.
class Writer {
    using Owner = std::unique_ptr<yyjson_mut_doc, decltype(&yyjson_mut_doc_free)>;
    Owner doc_{yyjson_mut_doc_new(nullptr), yyjson_mut_doc_free};
public:
    Writer() { if (!doc_) throw std::bad_alloc(); }
    yyjson_mut_val* object_value() { return checked(yyjson_mut_obj(doc_.get())); }
    void put(yyjson_mut_val* object, std::string_view key, yyjson_mut_val* value) {
        if (!yyjson_mut_obj_add(object, text(key), value)) throw std::bad_alloc();
    }
    yyjson_mut_val* text(std::string_view value) {
        return checked(yyjson_mut_strncpy(doc_.get(), value.data(), value.size()));
    }
    yyjson_mut_val* number(std::uint64_t value) { return checked(yyjson_mut_uint(doc_.get(), value)); }
    std::string finish(yyjson_mut_val* root) {
        yyjson_mut_doc_set_root(doc_.get(), root);
        std::size_t length{};
        std::unique_ptr<char, decltype(&std::free)> bytes(yyjson_mut_write(doc_.get(), 0, &length), std::free);
        if (!bytes) throw std::invalid_argument("Cannot encode metadata JSON");
        return {bytes.get(), length};
    }
private:
    static yyjson_mut_val* checked(yyjson_mut_val* value) {
        if (!value) throw std::bad_alloc();
        return value;
    }
};
}
ManifestVersions decode_manifest(std::string_view text, std::string_view project_id) {
    bounded("core.manifest", text.size(), max_manifest_bytes);
    const auto doc = document(text, project_id, true);
    ManifestVersions result;
    auto* files = yyjson_obj_get(yyjson_doc_get_root(doc.get()), "files");
    if (!files) return result;
    if (yyjson_is_obj(files)) bounded("core.manifest", yyjson_obj_size(files), max_manifest_files);
    object(files);
    auto iter = yyjson_obj_iter_with(files);
    while (auto* key = yyjson_obj_iter_next(&iter)) {
        auto* value = yyjson_obj_iter_get_val(key);
        object(value);
        const auto crc = unsigned_integer(yyjson_obj_get(value, "crc"));
        if (crc > std::numeric_limits<std::uint32_t>::max()) throw std::invalid_argument("CRC32 out of range");
        auto* publisher = yyjson_obj_get(value, "publisher");
        result.receive(std::string(string_value(key)), {
            unsigned_integer(yyjson_obj_get(value, "version")), unsigned_integer(yyjson_obj_get(value, "size")),
            static_cast<std::uint32_t>(crc), unsigned_integer(yyjson_obj_get(value, "updated_at")),
            publisher ? std::string(string_value(publisher)) : std::string{}});
    }
    return result;
}
std::string encode_manifest(const ManifestVersions& manifest, std::string_view project_id) {
    if (project_id.empty()) throw std::invalid_argument("Project id is required");
    Writer writer;
    auto* root = writer.object_value();
    auto* files = writer.object_value();
    writer.put(root, "format", writer.number(1));
    writer.put(root, "project_id", writer.text(project_id));
    writer.put(root, "files", files);
    for (const auto& [path, entry] : manifest.entries()) {
        auto* file = writer.object_value();
        writer.put(file, "version", writer.number(entry.version));
        writer.put(file, "size", writer.number(entry.size));
        writer.put(file, "crc", writer.number(entry.crc));
        writer.put(file, "updated_at", writer.number(entry.updated_at));
        writer.put(file, "publisher", writer.text(entry.publisher));
        writer.put(files, path, file);
    }
    return writer.finish(root);
}
VersionState decode_version_state(std::string_view text, std::string_view project_id) {
    bounded("core.version_state", text.size(), max_version_state_bytes);
    const auto doc = document(text, project_id, false);
    VersionState result;
    auto* versions = yyjson_obj_get(yyjson_doc_get_root(doc.get()), "versions");
    if (!versions) return result;
    if (yyjson_is_obj(versions)) bounded("core.version_state", yyjson_obj_size(versions), max_version_state_entries);
    object(versions);
    auto iter = yyjson_obj_iter_with(versions);
    while (auto* key = yyjson_obj_iter_next(&iter)) {
        result.note(std::string(string_value(key)), unsigned_integer(yyjson_obj_iter_get_val(key)));
    }
    return result;
}
std::string encode_version_state(const VersionState& state, std::string_view project_id) {
    if (project_id.empty()) throw std::invalid_argument("Project id is required");
    Writer writer;
    auto* root = writer.object_value();
    auto* versions = writer.object_value();
    writer.put(root, "format", writer.number(1));
    writer.put(root, "project_id", writer.text(project_id));
    writer.put(root, "versions", versions);
    for (const auto& [path, version] : state.entries()) writer.put(versions, path, writer.number(version));
    return writer.finish(root);
}
}
