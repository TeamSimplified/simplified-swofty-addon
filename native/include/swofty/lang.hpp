// SPDX-License-Identifier: MIT
#pragma once
#include <string>
#include <string_view>
#include <swofty/mixin.hpp>

// Runtime string overrides: change the text a lng_* key shows while the game runs, for every
// language or one language code (eng, fre, ...). The core copies the text; a change applies the
// next time the game looks the key up.
// Needs a core with API v8; older cores report unavailable.

namespace swofty::lang {

enum class Result { ok, late, unavailable, invalid };

namespace detail {

inline const SwoftyLang *table()
{
    return api && api->version >= 8 ? api->lang : nullptr;
}

inline Result result(int r)
{
    return r == SWOFTY_ASSET_OK ? Result::ok
           : r == SWOFTY_ASSET_LATE ? Result::late
           : r == SWOFTY_ASSET_UNAVAILABLE ? Result::unavailable
                                           : Result::invalid;
}

}

// The override library is usable (core v8, the lang feature on, the game's lookup found).
inline bool available()
{
    auto t = detail::table();
    return t && t->available();
}

// Make key show text in every language ("*") or in one language code.
inline Result set(std::string_view key, std::string_view text, std::string_view language = "*")
{
    auto t = detail::table();
    if (!t)
        return Result::unavailable;
    return detail::result(t->override_set(std::string(language).c_str(), std::string(key).c_str(), std::string(text).c_str()));
}

// Drop key's override for one language code, or for every language ("*").
inline Result clear(std::string_view key, std::string_view language = "*")
{
    auto t = detail::table();
    if (!t)
        return Result::unavailable;
    return detail::result(t->override_clear(std::string(language).c_str(), std::string(key).c_str()));
}

// Drop every override (of every addon).
inline Result clear_all()
{
    auto t = detail::table();
    if (!t)
        return Result::unavailable;
    return detail::result(t->override_clear_all());
}

// Send the current language to the game's language listeners, the notification the game sends
// after it loads a language. Call it on the flow thread, at most once per batch.
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
