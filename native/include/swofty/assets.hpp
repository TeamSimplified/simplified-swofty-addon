// SPDX-License-Identifier: MIT
#pragma once
#include <cstddef>
#include <cstdint>
#include <span>
#include <string>
#include <string_view>
#include <vector>
#include <swofty/mixin.hpp>

namespace swofty::assets {

enum class Result { ok, late, unavailable, invalid };

struct Fragment {
    std::string_view file;
    std::string_view fourcc = {};
    uint32_t flags = 0;
};

namespace detail {

inline const SwoftyAssets *table()
{
    return api && api->version >= 4 ? api->assets : nullptr;
}

inline Result result(int r)
{
    return r == SWOFTY_ASSET_OK ? Result::ok
           : r == SWOFTY_ASSET_LATE ? Result::late
           : r == SWOFTY_ASSET_UNAVAILABLE ? Result::unavailable
                                         : Result::invalid;
}

struct strings {
    std::vector<std::string> own;
    std::vector<const char *> ptrs;

    explicit strings(std::span<const std::string_view> list) : own(list.begin(), list.end())
    {
        for (auto &s : own)
            ptrs.push_back(s.c_str());
    }
};

}

inline bool available(std::string_view feature)
{
    auto t = detail::table();
    return t && t->available(std::string(feature).c_str());
}

inline Result register_group(std::string_view group, std::span<const std::string_view> erps,
                             std::span<const std::string_view> deps = {}, std::string_view templ = {})
{
    auto t = detail::table();
    if (!t)
        return Result::unavailable;
    detail::strings e(erps), d(deps);
    return detail::result(t->register_group(std::string(group).c_str(), e.ptrs.data(), e.ptrs.size(), d.ptrs.data(),
                                            d.ptrs.size(), std::string(templ).c_str()));
}

inline Result put_resource(std::string_view uri, std::span<const Fragment> fragments,
                           std::span<const std::string_view> erps = {})
{
    auto t = detail::table();
    if (!t)
        return Result::unavailable;
    std::vector<std::string> own;
    own.reserve(fragments.size() * 2);
    std::vector<SwoftyFragment> frags;
    for (auto &f : fragments) {
        own.emplace_back(f.file);
        const char *file = own.back().c_str();
        own.emplace_back(f.fourcc);
        frags.push_back({file, own.back().c_str(), f.flags});
    }
    detail::strings e(erps);
    return detail::result(
        t->put_resource(std::string(uri).c_str(), frags.data(), frags.size(), e.ptrs.data(), e.ptrs.size()));
}

inline Result db_set(std::string_view key, std::string_view type, std::string_view value)
{
    auto t = detail::table();
    if (!t)
        return Result::unavailable;
    return detail::result(t->db_set(std::string(key).c_str(), std::string(type).c_str(), std::string(value).c_str()));
}

inline Result db_set(uint64_t hash, uint8_t tag, std::span<const std::byte> value)
{
    auto t = detail::table();
    if (!t)
        return Result::unavailable;
    return detail::result(t->db_set_raw(hash, tag, value.data(), value.size()));
}

inline Result lang_set(std::string_view language, std::string_view key, std::string_view text)
{
    auto t = detail::table();
    if (!t)
        return Result::unavailable;
    return detail::result(
        t->lang_set(std::string(language).c_str(), std::string(key).c_str(), std::string(text).c_str()));
}

inline Result lang_table(std::string_view language, std::string_view file)
{
    auto t = detail::table();
    if (!t)
        return Result::unavailable;
    return detail::result(t->lang_table(std::string(language).c_str(), std::string(file).c_str()));
}

inline Result overlay_file(std::string_view game_path, std::string_view file)
{
    auto t = detail::table();
    if (!t)
        return Result::unavailable;
    return detail::result(t->overlay_file(std::string(game_path).c_str(), std::string(file).c_str()));
}

inline const char *name(Result r)
{
    return r == Result::ok ? "ok" : r == Result::late ? "late" : r == Result::unavailable ? "unavailable" : "invalid";
}

}
