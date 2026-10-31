// SPDX-License-Identifier: MIT
#pragma once
#include <cstddef>
#include <cstdint>
#include <span>
#include <string>
#include <string_view>
#include <vector>
#include <swofty/mixin.hpp>

// A native addon's own read/write storage under swofty/addons/<id>/. Paths are relative to that
// folder; the core rejects absolute paths, drive letters and any path that climbs above it. The
// calling DLL is identified the same way the assets API identifies it, so each addon reaches
// only its own folder. Writes are atomic (write to a temp file, then rename).

namespace swofty::storage {

enum class Result { ok, late, unavailable, invalid };

namespace detail {

inline const SwoftyStorage *table()
{
    return api && api->version >= 5 ? api->storage : nullptr;
}

inline Result result(int r)
{
    return r == SWOFTY_ASSET_OK ? Result::ok
           : r == SWOFTY_ASSET_LATE ? Result::late
           : r == SWOFTY_ASSET_UNAVAILABLE ? Result::unavailable
                                           : Result::invalid;
}

}

// The storage library is usable (feature on and implemented by this core).
inline bool available()
{
    auto t = detail::table();
    return t && t->available();
}

// Read a whole file into buf. *len is set to the file size; invalid when it exceeds cap.
inline Result read(std::string_view path, void *buf, size_t cap, size_t *len)
{
    auto t = detail::table();
    if (!t)
        return Result::unavailable;
    return detail::result(t->read(std::string(path).c_str(), buf, cap, len));
}

// Read a whole file into a string. Returns unavailable or invalid without touching out on error.
inline Result read(std::string_view path, std::string &out)
{
    auto t = detail::table();
    if (!t)
        return Result::unavailable;
    std::string p(path);
    uint64_t n = 0;
    if (Result sr = detail::result(t->size(p.c_str(), &n)); sr != Result::ok)
        return sr;
    std::string tmp(size_t(n), '\0');
    size_t got = 0;
    Result r = detail::result(t->read(p.c_str(), tmp.data(), tmp.size(), &got));
    if (r == Result::ok) {
        tmp.resize(got);
        out = std::move(tmp);
    }
    return r;
}

// Replace a file's contents atomically.
inline Result write(std::string_view path, std::span<const std::byte> data)
{
    auto t = detail::table();
    if (!t)
        return Result::unavailable;
    return detail::result(t->write(std::string(path).c_str(), data.data(), data.size()));
}

inline Result write(std::string_view path, std::string_view data)
{
    return write(path, std::span<const std::byte>(reinterpret_cast<const std::byte *>(data.data()), data.size()));
}

// Append to a file, creating it when absent.
inline Result append(std::string_view path, std::span<const std::byte> data)
{
    auto t = detail::table();
    if (!t)
        return Result::unavailable;
    return detail::result(t->append(std::string(path).c_str(), data.data(), data.size()));
}

inline Result append(std::string_view path, std::string_view data)
{
    return append(path, std::span<const std::byte>(reinterpret_cast<const std::byte *>(data.data()), data.size()));
}

// Delete a file.
inline Result remove(std::string_view path)
{
    auto t = detail::table();
    if (!t)
        return Result::unavailable;
    return detail::result(t->remove(std::string(path).c_str()));
}

// File size in bytes.
inline Result size(std::string_view path, uint64_t *out)
{
    auto t = detail::table();
    if (!t)
        return Result::unavailable;
    return detail::result(t->size(std::string(path).c_str(), out));
}

// List a directory into buf as newline-separated names. *len is the bytes written; invalid when
// the listing exceeds cap.
inline Result list(std::string_view dir, char *buf, size_t cap, size_t *len)
{
    auto t = detail::table();
    if (!t)
        return Result::unavailable;
    return detail::result(t->list(std::string(dir).c_str(), buf, cap, len));
}

inline const char *name(Result r)
{
    return r == Result::ok ? "ok" : r == Result::late ? "late" : r == Result::unavailable ? "unavailable" : "invalid";
}

}
