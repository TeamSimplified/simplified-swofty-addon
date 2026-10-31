// SPDX-License-Identifier: MIT
#pragma once
#include <swofty/registry.hpp>
#include <swofty/systems/forward.hpp>

// Power: the DRS rules a Grand Prix or Time Trial box names in its rules (sessions::Rules::drs). Each rule
// becomes a DRS allowed-lap group in the game database at boot; the box's rule applies when the box is
// pressed. Power units, suppliers and component rules come later.

namespace swofty::power {

struct DrsRule {
    static constexpr Version version{1, 0};
    int race_start = 1;
    int red_flag_restart = 1;
    int safety_car_end = 1;
    json::Value extra;
};
SWOFTY_RECORD(DrsRule, race_start, red_flag_restart, safety_car_end)

inline Registry<DrsRule> drs_rules{"rules.drs", {.applies = Applies::boot}};

}
