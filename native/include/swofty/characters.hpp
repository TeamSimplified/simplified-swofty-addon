// SPDX-License-Identifier: MIT
#pragma once
#include <string>
#include <string_view>
#include <swofty/mixin.hpp>

// Per-career character overrides: in careers of one data group, show another character's portrait,
// names and job on a character's UI ref, play a speaker's lines silently, and set runtime string
// overrides that hold only while that career is loaded. The core owns the hooks, so several addons
// can register for different data groups or characters. Needs a core with the characters table;
// older cores report unavailable.

namespace swofty::characters {

enum class Result { ok, late, unavailable, invalid };

namespace detail {

inline const SwoftyCharacters *table()
{
    return api && api->version >= 8 ? api->characters : nullptr;
}

inline Result result(int r)
{
    return r == SWOFTY_ASSET_OK ? Result::ok
           : r == SWOFTY_ASSET_LATE ? Result::late
           : r == SWOFTY_ASSET_UNAVAILABLE ? Result::unavailable
                                           : Result::invalid;
}

inline std::string z(std::string_view s)
{
    return std::string(s);
}

}

// The library is usable (the core has the table and its hooks are installed or not needed yet).
inline bool available()
{
    auto t = detail::table();
    return t && t->available();
}

// The loaded career's data group, or "" with no career loaded.
inline std::string career_group()
{
    auto t = detail::table();
    char buf[128] = {};
    if (!t || t->career_group(buf, sizeof buf) != SWOFTY_ASSET_OK)
        return {};
    return buf;
}

// In careers of group, the character row named character shows the row named replacement
// (portrait, names, job), under character's stat_id.
inline Result swap(std::string_view group, std::string_view character, std::string_view replacement)
{
    auto t = detail::table();
    if (!t)
        return Result::unavailable;
    return detail::result(
        t->swap(detail::z(group).c_str(), detail::z(character).c_str(), detail::z(replacement).c_str()));
}

// Drop the swap of character in group.
inline Result restore(std::string_view group, std::string_view character)
{
    auto t = detail::table();
    if (!t)
        return Result::unavailable;
    return detail::result(t->swap(detail::z(group).c_str(), detail::z(character).c_str(), nullptr));
}

// In careers of group, lines of speaker (agent, agent_cutscene, ...) play at speech volume 0, with
// their subtitles and callbacks.
inline Result mute(std::string_view group, std::string_view speaker, bool on = true)
{
    auto t = detail::table();
    if (!t)
        return Result::unavailable;
    return detail::result(t->mute(detail::z(group).c_str(), detail::z(speaker).c_str(), on ? 1 : 0));
}

// In careers of group, key shows text in one language code, or in every language ("*").
inline Result string(std::string_view group, std::string_view key, std::string_view text, std::string_view language = "*")
{
    auto t = detail::table();
    if (!t)
        return Result::unavailable;
    return detail::result(t->string_set(detail::z(group).c_str(), detail::z(language).c_str(), detail::z(key).c_str(),
                                        detail::z(text).c_str()));
}

// Drop the override of key in group for one language code, or for "*".
inline Result clear_string(std::string_view group, std::string_view key, std::string_view language = "*")
{
    auto t = detail::table();
    if (!t)
        return Result::unavailable;
    return detail::result(
        t->string_set(detail::z(group).c_str(), detail::z(language).c_str(), detail::z(key).c_str(), nullptr));
}

// Every string of a JSON object of language objects, {"eng": {"lng_key": "text", ...}, ...}, for
// careers of group.
inline Result strings_json(std::string_view group, std::string_view json)
{
    auto t = detail::table();
    if (!t)
        return Result::unavailable;
    return detail::result(t->strings_json(detail::z(group).c_str(), json.data(), json.size()));
}

// Drop every swap, mute and string this addon registered for group.
inline Result clear(std::string_view group)
{
    auto t = detail::table();
    if (!t)
        return Result::unavailable;
    return detail::result(t->clear(detail::z(group).c_str()));
}

// Rebuild the game's character list now, so a swap registered while a career is loaded shows.
// Call it on the game thread.
inline Result refresh()
{
    auto t = detail::table();
    if (!t)
        return Result::unavailable;
    return detail::result(t->refresh());
}

inline const char *name(Result r)
{
    static const char *names[] = {"ok", "late", "unavailable", "invalid"};
    return names[static_cast<int>(r)];
}

}
