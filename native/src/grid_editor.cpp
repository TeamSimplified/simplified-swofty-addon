// SPDX-License-Identifier: AGPL-3.0-or-later
//
// The Grid Editor's seats on the grids the game makes from registry records, which don't exist when the
// build's database edits apply. Simplified's build step puts them in profile/grid-editor.json, in
// Simplified's folder in the game, and this layers them on those records for the profile that played,
// one layer a record: the core keeps one layer an add-on and record, so every seat on a record goes in one
// layer. A picked driver goes on its seat as "<picked>|<the seat's driver>", so the core takes the first of
// them that resolves and a pick that doesn't falls back to the seat's own driver, and only that seat.
//
// "seats": Progressive Career's line-ups. Progressive registers each year as a career.season record,
// swofty.swofty-seasons-progressive/2016 to /2026 (swofty.progressive/ on the older add-on), and seats its
// careers from it; the drivers edited on the Classic Seasons, F1 2025 and F1 2026 grids are those seasons' drivers too. By year and career slot, a driver
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
#include <vector>

#include <swofty/json.hpp>
#include <swofty/registry.hpp>
#include <swofty/storage.hpp>
#include <swofty/systems/career.hpp>
#include <swofty/systems/content.hpp>

using namespace swofty;

using Driver = Ref<drivers::Identity>;

static constexpr std::array<const char *, 2> kProgressive = {"swofty.swofty-seasons-progressive", "swofty.progressive"};

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

static Driver pick(const std::string &picked, const Driver &current)
{
    Driver d(picked);
    return current.empty() || current.id() == picked ? d : d.or_else(current);
}

struct SeasonSeat {
    int slot;
    size_t seat;
    std::string driver;
};

struct PackageSeat {
    std::string team;
    size_t seat;
    std::string driver, label, look;
    int number;
};

static void progressive_seats(const json::Value &years)
{
    for (const auto &[year, slots] : members(years)) {
        std::vector<SeasonSeat> edits;
        for (const auto &[slot, seats] : members(slots)) {
            int n = number_of(slot);
            const json::Array *list = seats.if_array();
            if (n < 1 || !list)
                continue;
            for (size_t seat = 0; seat < 2 && seat < list->size(); ++seat) {
                std::string id = text_of((*list)[seat]);
                if (!id.empty())
                    edits.push_back({n, seat, std::move(id)});
            }
        }
        if (edits.empty())
            continue;
        for (const char *addon : kProgressive)
            career::seasons.layer(career::seasons[std::string(addon) + "/" + year], [year, edits, addon](career::Season &s) {
                size_t applied = 0, skipped = 0;
                for (const SeasonSeat &e : edits) {
                    career::SeasonTeam *team = s.teams.find(e.slot);
                    if (!team) {
                        ++skipped;
                        continue;
                    }
                    team->seats[e.seat] = pick(e.driver, team->seats[e.seat]);
                    ++applied;
                }
                swofty::log("[simplified] grid editor: %s/%s: %zu seats set, %zu skipped for structure", addon, year.c_str(),
                            applied, skipped);
            }, Scope::profile);
    }
}

static void package_seats(const json::Value &packages)
{
    for (const auto &[package, teams] : members(packages)) {
        std::vector<PackageSeat> edits;
        for (const auto &[key, seats] : members(teams))
            for (const auto &[at, change] : members(seats)) {
                int seat = number_of(at);
                if (seat < 0)
                    continue;
                edits.push_back({key, static_cast<size_t>(seat), text_of(change["driver"]), text_of(change["label"]),
                                 text_of(change["look"]),
                                 change["number"].is_number() ? static_cast<int>(change["number"].as_int()) : 0});
            }
        if (edits.empty())
            continue;
        content::packages.layer(content::packages[package], [package, edits](content::Package &p) {
            size_t applied = 0, skipped = 0;
            for (const PackageSeat &e : edits) {
                content::Team *team = p.teams.find(e.team);
                if (!team || e.seat >= team->drivers.size()) {
                    ++skipped;
                    continue;
                }
                content::Seat &s = team->drivers[e.seat];
                if (!e.driver.empty())
                    s.driver = pick(e.driver, s.driver);
                if (e.number > 0)
                    s.number = e.number;
                if (!e.look.empty() && !e.label.empty() && s.look.empty())
                    p.extra["database"]["looks"][e.team + "/" + e.label] = json::Value(e.look);
                ++applied;
            }
            swofty::log("[simplified] grid editor: %s: %zu seats set, %zu skipped for structure", package.c_str(), applied,
                        skipped);
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
