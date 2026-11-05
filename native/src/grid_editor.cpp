// SPDX-License-Identifier: AGPL-3.0-or-later
//
// The Grid Editor's seats on the grids the game makes from registry records, which don't exist when the
// build's database edits apply. Simplified's build step puts them in profile/grid-editor.json, in
// Simplified's folder in the game, and this layers them on those records for the profile that played,
// one layer a seat: a layer whose reference doesn't resolve is dropped whole, and so takes only its seat.
//
// "seats": Progressive Career's line-ups. Progressive registers each year as a career.season record,
// swofty.progressive/2016 to /2026, and seats its careers from it; the drivers edited on the Classic
// Seasons, F1 2025 and F1 2026 grids are those seasons' drivers too. By year and career slot, a driver
// for each seat, "" for a seat that keeps Progressive's:
//   { "2016": { "4": ["", "person/stoffel_vandoorne|game/stoffel_vandoorne"] } }
// A career keeps the seats of the seasons it has already started.
//
// "packages": the grids addons define as content.package records, such as each Classic Seasons year,
// whose entrant lists the core makes from the record when the game starts. By record, team key and seat:
//   { "swofty.classic-seasons-2022/f1_2022": { "williams": { "1": { "driver":
//     "swofty.classic-seasons/pascal_wehrlein", "label": "pascal_wehrlein",
//     "look": "2016-cs_manor_pascal_wehrlein", "number": 94 } } } }
// The driver and number go on the seat. The look goes in the package's own looks ("database.looks",
// "<team>/<driver label>"), where the core finds a seat's look: a look reference must resolve, and the
// rows the looks are aren't records.

#include <array>
#include <charconv>
#include <string>

#include <swofty/json.hpp>
#include <swofty/registry.hpp>
#include <swofty/storage.hpp>
#include <swofty/systems/career.hpp>
#include <swofty/systems/content.hpp>

using namespace swofty;

using Driver = Ref<drivers::Identity>;

static const json::Object &members(const json::Value &v)
{
    static const json::Object none;
    const json::Object *o = v.if_object();
    return o ? *o : none;
}

static int number_of(std::string_view key)
{
    int n = 0;
    auto [end, err] = std::from_chars(key.data(), key.data() + key.size(), n);
    return err == std::errc() && end == key.data() + key.size() ? n : -1;
}

static std::string text_of(const json::Value &v)
{
    const std::string *s = v.if_string();
    return s ? *s : std::string();
}

static void progressive_seats(const json::Value &years)
{
    for (const auto &[year, slots] : members(years))
        for (const auto &[slot, seats] : members(slots)) {
            int n = number_of(slot);
            const json::Array *list = seats.if_array();
            if (n < 1 || !list)
                continue;
            for (size_t seat = 0; seat < 2 && seat < list->size(); ++seat) {
                std::string id = text_of((*list)[seat]);
                if (id.empty())
                    continue;
                career::seasons.layer(career::seasons["swofty.progressive/" + year], [n, seat, id](career::Season &s) {
                    if (career::SeasonTeam *team = s.teams.find(n))   // a slot Progressive doesn't have is skipped
                        team->seats[seat] = Driver(id);
                }, Scope::profile);
            }
        }
}

static void package_seats(const json::Value &packages)
{
    for (const auto &[package, teams] : members(packages))
        for (const auto &[key, seats] : members(teams))
            for (const auto &[at, change] : members(seats)) {
                int seat = number_of(at);
                if (seat < 0)
                    continue;
                std::string driver = text_of(change["driver"]), label = text_of(change["label"]),
                            look = text_of(change["look"]);
                int number = change["number"].is_number() ? static_cast<int>(change["number"].as_int()) : 0;
                content::packages.layer(content::packages[package], [key, seat, driver, label, look,
                                                                     number](content::Package &p) {
                    content::Team *team = p.teams.find(key);
                    if (!team || static_cast<size_t>(seat) >= team->drivers.size())
                        return;                                 // a team or seat the record doesn't have
                    content::Seat &s = team->drivers[static_cast<size_t>(seat)];
                    if (!driver.empty())
                        s.driver = Driver(driver);
                    if (number > 0)
                        s.number = number;
                    if (!look.empty() && !label.empty() && s.look.empty())
                        p.extra["database"]["looks"][key + "/" + label] = json::Value(look);
                }, Scope::profile);
            }
}

SWOFTY_REGISTER
{
    std::string text;
    if (storage::read("profile/grid-editor.json", text) != storage::Result::ok)
        return;                                             // no build has put any seats here
    const json::Value edits = json::parse(text);
    progressive_seats(edits["seats"]);
    package_seats(edits["packages"]);
}

SWOFTY_ADDON()
