// SPDX-License-Identifier: AGPL-3.0-or-later
//
// The Grid Editor's seats for Progressive Career. Progressive registers each year's line-up as a
// career.season record, swofty.progressive/2016 to /2026, and seats its careers from it. The drivers
// edited on the Classic Seasons, F1 2025 and F1 2026 grids are those seasons' drivers too: Simplified's
// build step puts them in profile/progressive.json, in Simplified's folder in the game, and this layers
// them on Progressive's seasons for the profile that played.
//
// { "seats": { "2016": { "10": ["", "game/stoffel_vandoorne"] } } }: by year and career slot, a
// driver for each seat, "" for a seat that keeps Progressive's. A season or slot Progressive doesn't
// have is skipped; a career keeps the seats of the seasons it has already started.

#include <array>
#include <charconv>
#include <map>
#include <string>

#include <swofty/json.hpp>
#include <swofty/registry.hpp>
#include <swofty/storage.hpp>
#include <swofty/systems/career.hpp>

using namespace swofty;

using Driver = Ref<drivers::Identity>;

static const json::Object &members(const json::Value &v)
{
    static const json::Object none;
    const json::Object *o = v.if_object();
    return o ? *o : none;
}

static int slot_number(std::string_view key)
{
    int n = 0;
    auto [end, err] = std::from_chars(key.data(), key.data() + key.size(), n);
    return err == std::errc() && end == key.data() + key.size() ? n : 0;
}

static Driver seat_driver(const json::Value &seats, size_t seat)
{
    const json::Array *list = seats.if_array();
    const std::string *id = list && seat < list->size() ? (*list)[seat].if_string() : nullptr;
    return id && !id->empty() ? Driver(*id) : Driver();
}

SWOFTY_REGISTER
{
    std::string text;
    if (storage::read("profile/progressive.json", text) != storage::Result::ok)
        return;                                             // no build has put any seats here
    const json::Value edits = json::parse(text);

    for (const auto &[year, slots] : members(edits["seats"])) {
        std::map<int, std::array<Driver, 2>> grid;
        for (const auto &[slot, seats] : members(slots))
            if (int n = slot_number(slot))
                grid[n] = {seat_driver(seats, 0), seat_driver(seats, 1)};
        if (grid.empty())
            continue;

        career::seasons.layer(career::seasons["swofty.progressive/" + year], [grid](career::Season &s) {
            for (const auto &[slot, seats] : grid)
                if (career::SeasonTeam *team = s.teams.find(slot))
                    for (size_t seat = 0; seat < seats.size(); ++seat)
                        if (!seats[seat].empty())
                            team->seats[seat] = seats[seat];
        }, Scope::profile);
    }
}

SWOFTY_ADDON()
