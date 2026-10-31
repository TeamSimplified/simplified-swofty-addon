// SPDX-License-Identifier: MIT
#pragma once
#include <string>
#include <string_view>
#include <swofty/mixin.hpp>

// The game's own message popup with your text: a title, a body and one OK button, drawn by the same
// persistent popup the game uses for its errors and confirmations. show() queues the popup from
// any thread; the core shows it on the game's flow thread, on top of the current screen.

namespace swofty::popup {

enum class Result { ok, late, unavailable, invalid };

namespace detail {

inline const SwoftyPopup *table()
{
    return api && api->version >= 7 ? api->popup : nullptr;
}

inline Result result(int r)
{
    return r == SWOFTY_ASSET_OK ? Result::ok
           : r == SWOFTY_ASSET_LATE ? Result::late
           : r == SWOFTY_ASSET_UNAVAILABLE ? Result::unavailable
                                           : Result::invalid;
}

}

// The popup library is usable: the core found the game's message system on this build.
inline bool available()
{
    auto t = detail::table();
    return t && t->available();
}

// Queue a popup. ok once queued; invalid for an empty title and body; unavailable on an older core
// or a build where the message system did not resolve. UTF-8, line breaks as '\n'.
inline Result show(std::string_view title, std::string_view body)
{
    auto t = detail::table();
    if (!t)
        return Result::unavailable;
    return detail::result(t->show(std::string(title).c_str(), std::string(body).c_str()));
}

inline const char *name(Result r)
{
    return r == Result::ok ? "ok" : r == Result::late ? "late" : r == Result::unavailable ? "unavailable" : "invalid";
}

}
