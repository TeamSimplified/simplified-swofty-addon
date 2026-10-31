// SPDX-License-Identifier: MIT
#pragma once
#include <cstdint>
#include <string>
#include <string_view>
#include <swofty/mixin.hpp>

// Career wizard control for one career data group (the data group of the career content tile the
// wizard opens from): start in F1, open another route from the tile, replace wizard steps, force
// an option, change the confirm step's route and hide Choose Driver tabs. The core owns the hooks,
// so several addons can register for different data groups. Needs a core with the wizard table;
// older cores report unavailable.

namespace swofty::career::wizard {

enum class Result { ok, late, unavailable, invalid };

// Wizard step bytes (Game_CareerWizard_PushNextStep).
namespace step {
inline constexpr uint32_t experience = 3, season_length = 4, settings = 5, confirm = 6, classic_drivers = 14,
                          my_team_entry = 15, team_icons = 16, classic_drivers_settings = 17;
}

// Wizard option bytes, the choices the player makes. custom_season (10) is the option whose next step is the
// track select; its label is not confirmed in the KB.
namespace option {
inline constexpr uint32_t driver = 2, my_team = 3, skip_to_f1 = 6, start_in_f2 = 7, full_season = 9,
                          custom_season = 10, recommended_settings = 0xb, custom_settings = 0xc, team_icon = 0x1e;
}

// Routes the stock wizard uses.
inline constexpr char route_season_length[] = "career_wizard_driver_career_to_f1";
inline constexpr char route_driver_type[] = "driver_career_driver_type_select";
inline constexpr char route_driver_select[] = "driver_career_driver_select";

namespace detail {

inline const SwoftyCareerWizard *table()
{
    return api && api->version >= 8 ? api->career_wizard : nullptr;
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

// The library is usable (the core has the table and a wizard hook is installed or none was needed yet).
inline bool available()
{
    auto t = detail::table();
    return t && t->available();
}

// The data group of the last career tile the player pressed, or "".
inline std::string group()
{
    auto t = detail::table();
    char buf[128] = {};
    if (!t || t->group(buf, sizeof buf) != SWOFTY_ASSET_OK)
        return {};
    return buf;
}

// New careers of group start in F1: no Skip to F1 / Start in F2 step and no team-icons step.
inline Result start_in_f1(std::string_view group, bool on = true)
{
    auto t = detail::table();
    if (!t)
        return Result::unavailable;
    return detail::result(t->starts_in_f1(detail::z(group).c_str(), on ? 1 : 0));
}

// The tile of group opens route to instead of from ("" for whatever route the tile picks). The
// route must exist in the front end's routes (add it with an erp op).
inline Result tile_route(std::string_view group, std::string_view to, std::string_view from = route_season_length)
{
    auto t = detail::table();
    if (!t)
        return Result::unavailable;
    return detail::result(t->tile_route(detail::z(group).c_str(), detail::z(from).c_str(), detail::z(to).c_str()));
}

// Where the wizard of group would push step, it pushes with.
inline Result replace_step(std::string_view group, uint32_t from, uint32_t with)
{
    auto t = detail::table();
    if (!t)
        return Result::unavailable;
    return detail::result(t->replace_step(detail::z(group).c_str(), from, with));
}

// No Add Driver Icons To Market step: the settings go straight to the confirm step.
inline Result skip_driver_icons(std::string_view group, bool on = true)
{
    return replace_step(group, step::classic_drivers, on ? step::confirm : step::classic_drivers);
}

// Where the player picks option, the wizard of group records with.
inline Result force_option(std::string_view group, uint32_t from, uint32_t with)
{
    auto t = detail::table();
    if (!t)
        return Result::unavailable;
    return detail::result(t->force_option(detail::z(group).c_str(), from, with));
}

// The season length step of group records full_season (or custom_season) whatever the player picks.
inline Result season_length(std::string_view group, uint32_t choice)
{
    Result a = force_option(group, option::full_season, choice);
    Result b = force_option(group, option::custom_season, choice);
    return a != Result::ok ? a : b;
}

// The confirm step of group opens route to instead of from ("" for any route).
inline Result confirm_route(std::string_view group, std::string_view to, std::string_view from = route_driver_type)
{
    auto t = detail::table();
    if (!t)
        return Result::unavailable;
    return detail::result(t->confirm_route(detail::z(group).c_str(), detail::z(from).c_str(), detail::z(to).c_str()));
}

// No Official Driver / Custom Driver step: the confirm step opens Choose Driver.
inline Result skip_driver_type(std::string_view group, bool on = true)
{
    auto t = detail::table();
    if (!t)
        return Result::unavailable;
    return detail::result(
        t->confirm_route(detail::z(group).c_str(), route_driver_type, on ? route_driver_select : nullptr));
}

// The Choose Driver header of group's wizard has no tab (header_f1, header_selected_team_icon,
// header_f2, header_icon).
inline Result hide_driver_tab(std::string_view group, std::string_view tab, bool on = true)
{
    auto t = detail::table();
    if (!t)
        return Result::unavailable;
    return detail::result(t->hide_driver_tab(detail::z(group).c_str(), detail::z(tab).c_str(), on ? 1 : 0));
}

// Drop every wizard entry this addon registered for group.
inline Result clear(std::string_view group)
{
    auto t = detail::table();
    if (!t)
        return Result::unavailable;
    return detail::result(t->clear(detail::z(group).c_str()));
}

inline const char *name(Result r)
{
    static const char *names[] = {"ok", "late", "unavailable", "invalid"};
    return names[static_cast<int>(r)];
}

}
