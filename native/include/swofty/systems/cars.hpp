// SPDX-License-Identifier: MIT
#pragma once
#include <array>
#include <atomic>
#include <optional>
#include <swofty/events.hpp>
#include <swofty/registry.hpp>
#include <swofty/systems/control.hpp>
#include <swofty/systems/forward.hpp>

// Cars: vehicle rows and their packages (boot), each car's handling file (applied as the car is built at
// every session load; the 1.6.0 core stores handling records and applies nothing from them). Handling parts are named after the handling
// file's sections and their fields after its keys, in snake_case (FrontCoeffLiftMultiplier ->
// front_coeff_lift_multiplier); a key this header doesn't name is kept in the part's `extra`. cars::live
// is the cars on track, read and written through the core's car operations (SwoftyCoreCarOps): cars()
// lists them, get/set/scale read and write one live::Field, decoded and re-encoded by the core. Without
// those operations (an older core, or an exe that isn't the build the field offsets were read from)
// live::available() is false and live::cars() is empty. A live::Car is a trivially copyable handle (the
// car's index and the game's car), so event payloads carry it as is. control() reads the car's controller
// mode and flags fresh from the core (nothing while the game rebuilds its controller sets, or on a core
// without session operations). cars::live is experimental: nothing in it has run in game yet, and the final
// drive and gear ratio fields depend on the gearbox's use_setup_ratios byte (+0x540), not yet read at runtime.

namespace swofty::cars {

using Result = swofty::Result;

enum class Class { f1, f1_26, f2, classic, my_team, safety_car };
SWOFTY_ENUM(Class, f1, f1_26, f2, classic, my_team, safety_car)
enum class Department : int32_t { chassis, aero, powertrain, durability, tyres };
SWOFTY_ENUM(Department, chassis, aero, powertrain, durability, tyres)

struct Package {
    static constexpr Version version{1, 0};
    Ref<Package> donor;
    std::string resource_path;
    std::string ai_package_folder;
    std::string driver_animation_set;
    bool halo = true;
    bool aero_2026 = false;
    Ref<Package> setups_from;
    StatId stat_id = StatId::automatic;
    json::Value extra;
};
SWOFTY_RECORD(Package, donor, resource_path, ai_package_folder, driver_animation_set, halo, aero_2026, setups_from,
              stat_id)

struct AiSlot {
    Ref<ai::TrackPack> track_pack;
    std::string asset_filename = "auto";
};
SWOFTY_RECORD(AiSlot, track_pack, asset_filename)

struct Vehicle {
    static constexpr Version version{1, 0};
    Ref<Package> package;
    std::string label;
    Ref<Vehicle> donor;
    Path model;
    Ref<sounds::EngineSound> engine_sound;
    Ref<liveries::Livery> livery;
    Ref<Handling> handling;
    Ref<power::Unit> power_unit;
    AiSlot ai;
    Class performance_class = Class::f1;
    int performance_tier = 0;
    float tyre_wear_scale = 1.0f;
    StatId stat_id = StatId::automatic;
    json::Value extra;
};
SWOFTY_RECORD(Vehicle, package, label, donor, model, engine_sound, livery, handling, power_unit, ai, performance_class,
              performance_tier, tyre_wear_scale, stat_id)

struct WingDamage {
    float speed_threshold = 3, damage_scale = 0.0001f, chassis_damage_multiple = 0.3f, side_detach_scale = 0.5f,
          full_detach_scale = 1;
    json::Value extra;
};
SWOFTY_RECORD(WingDamage, speed_threshold, damage_scale, chassis_damage_multiple, side_detach_scale, full_detach_scale)

struct SuspensionDamage {
    float speed_threshold = 7, front_damage_scale = 0.001f, rear_damage_scale = 0.0001f, wheel_detach_scale = 1,
          parts_detach_scale = 1.5f, linear_fracture_limit = 50, angular_fracture_limit = 30;
    json::Value extra;
};
SWOFTY_RECORD(SuspensionDamage, speed_threshold, front_damage_scale, rear_damage_scale, wheel_detach_scale,
              parts_detach_scale, linear_fracture_limit, angular_fracture_limit)

struct Chassis {
    float mass = 0, moi_x = 0, moi_y = 0, moi_z = 0, com_y = 0, com_z = 0;
    json::Value extra;
};
SWOFTY_RECORD(Chassis, mass, moi_x, moi_y, moi_z, com_y, com_z)

struct F1Aero {
    float front_coeff_lift_multiplier = 1, rear_coeff_lift_multiplier = 1, body_drag_coeff_min = 0,
          body_drag_coeff_max = 0;
    Curve front_coeff_lift_vs_ride_height, rear_coeff_lift_vs_ride_height;
    json::Value extra;
};
SWOFTY_RECORD(F1Aero, front_coeff_lift_multiplier, rear_coeff_lift_multiplier, body_drag_coeff_min, body_drag_coeff_max,
              front_coeff_lift_vs_ride_height, rear_coeff_lift_vs_ride_height)

struct CoreAero {
    bool slip_streaming = true, slip_stream_while_braking = false;
    json::Value extra;
};
SWOFTY_RECORD(CoreAero, slip_streaming, slip_stream_while_braking)

struct Engine {
    Curve revs_power_spline;
    float idle_rotation_rate = 0, limiter_rotation_rate = 0;
    float mass = 0, inertia = 0;
    float wear_ice_rate = 0, wear_tc_rate = 0, wear_mguh_rate = 0, wear_ce_rate = 0;
    json::Value extra;
};
SWOFTY_RECORD(Engine, revs_power_spline, idle_rotation_rate, limiter_rotation_rate, mass, inertia, wear_ice_rate,
              wear_tc_rate, wear_mguh_rate, wear_ce_rate)

struct FuelTank {
    float fuel_mass = 0, fuel_bsfc_min = 0;
    json::Value extra;
};
SWOFTY_RECORD(FuelTank, fuel_mass, fuel_bsfc_min)

struct FuelEcu {
    std::array<float, 4> fuel_burn_scale{}, fuel_power_scale{}, fuel_revs_scale{}, temperature_scale{};
    json::Value extra;
};
SWOFTY_RECORD(FuelEcu, fuel_burn_scale, fuel_power_scale, fuel_revs_scale, temperature_scale)

struct ErsMode {
    std::vector<float> deploy;
    json::Value extra;
};
SWOFTY_RECORD(ErsMode, deploy)

struct Ers {
    float max_deploy_power = 0, energy_store_capacity = 0;
    float exhaust_power_coefficient = 0, overtake_power_efficiency = 0, max_brake_torque_mguk = 0;
    std::array<ErsMode, 4> modes;
    json::Value extra;
};
SWOFTY_RECORD(Ers, max_deploy_power, energy_store_capacity, exhaust_power_coefficient, overtake_power_efficiency,
              max_brake_torque_mguk, modes)

struct Tuning {
    float differential_on_throttle_multiple_min = 0, brake_bias_max = 0;
    json::Value extra;
};
SWOFTY_RECORD(Tuning, differential_on_throttle_multiple_min, brake_bias_max)

struct Plank {
    float offset_x = 0, offset_y = 0, offset_z = 0, extent_x = 0, extent_y = 0, extent_z = 0;
    json::Value extra;
};
SWOFTY_RECORD(Plank, offset_x, offset_y, offset_z, extent_x, extent_y, extent_z)

struct Section {
    json::Value extra;
};
SWOFTY_RECORD(Section)

struct UpgradeChange {
    std::string section, key;
    char op = '=';
    float value = 0;
};
SWOFTY_RECORD(UpgradeChange, section, key, op, value)

struct RadUpgrade {
    std::string name;
    std::vector<UpgradeChange> changes;
    RadUpgrade &set(std::string_view section, std::string_view key, char op, float value)
    {
        for (auto &c : changes)
            if (c.section == section && c.key == key) {
                c.op = op;
                c.value = value;
                return *this;
            }
        changes.push_back({std::string(section), std::string(key), op, value});
        return *this;
    }
};
SWOFTY_RECORD(RadUpgrade, name, changes)

struct Handling {
    static constexpr Version version{1, 0};
    Ref<Handling> from;
    Path vtf;
    Class cls = Class::f1;
    float damage_scale = 1.0f;
    Chassis chassis;
    F1Aero f1_aero;
    CoreAero core_aero;
    WingDamage wing_damage;
    SuspensionDamage suspension_damage;
    Section tyre_damage;
    Section body_damage;
    Section tyres;
    Engine engine;
    Section gearbox;
    FuelTank fuel_tank;
    FuelEcu fuel_ecu;
    Section brake_ecu;
    Section steering;
    Section differential;
    Section suspension;
    Tuning tuning;
    Section assists;
    Section turbo;
    Ers ers;
    Plank plank;
    std::vector<RadUpgrade> rad_upgrades;
    json::Value extra;

    RadUpgrade &upgrade(std::string_view name)
    {
        for (auto &u : rad_upgrades)
            if (u.name == name)
                return u;
        rad_upgrades.push_back({std::string(name), {}});
        return rad_upgrades.back();
    }
};
SWOFTY_RECORD(Handling, from, vtf, cls, damage_scale, chassis, f1_aero, core_aero, wing_damage, suspension_damage,
              tyre_damage, body_damage, tyres, engine, gearbox, fuel_tank, fuel_ecu, brake_ecu, steering, differential,
              suspension, tuning, assists, turbo, ers, plank, rad_upgrades)

struct Part {
    std::string node;
    float detach_at = 0;
    bool debris = true;
};
SWOFTY_RECORD(Part, node, detach_at, debris)

inline Registry<Vehicle> vehicles{"vehicle", {.applies = Applies::boot}};
inline Registry<Package> packages{"vehicle.package", {.applies = Applies::boot}};
inline Registry<Handling> handling{"vehicle.handling", {.applies = Applies::session}};

inline Result reset_upgrades(int) { return Result::unavailable; }

namespace live {

// The fields set() can write, by part. Units and ranges are the handling file's; set() refuses a value
// outside the range with Result::invalid.
enum class Field : uint32_t {
    steering_lock = SWOFTY_CAR_STEERING_LOCK,                  // Steering/MaxSteeringLock, rad, 0.01 .. 1.5708
    clutch_torque = SWOFTY_CAR_CLUTCH_TORQUE,                  // GearBox/MaxClutchTorque, multiple of engine torque
    diff_on_throttle = SWOFTY_CAR_DIFF_ON_THROTTLE,            // setup differential on throttle, 0 .. 1
    diff_off_throttle = SWOFTY_CAR_DIFF_OFF_THROTTLE,          // setup differential off throttle, 0 .. 1
    turbo_max_boost = SWOFTY_CAR_TURBO_MAX_BOOST,              // Turbo/MaxBoost, bar
    fuel_mass = SWOFTY_CAR_FUEL_MASS,                          // kg in the tank, up to its capacity
    wing_speed_threshold = SWOFTY_CAR_WING_SPEED_THRESHOLD,    // F1WingDamage, read on every hit
    wing_damage_scale = SWOFTY_CAR_WING_DAMAGE_SCALE,
    wing_chassis_damage_multiple = SWOFTY_CAR_WING_CHASSIS_DAMAGE_MULTIPLE,
    wing_side_detach_scale = SWOFTY_CAR_WING_SIDE_DETACH_SCALE,
    wing_full_detach_scale = SWOFTY_CAR_WING_FULL_DETACH_SCALE,
    susp_speed_threshold = SWOFTY_CAR_SUSP_SPEED_THRESHOLD,    // F1SuspensionDamage, read on every hit
    susp_front_damage_scale = SWOFTY_CAR_SUSP_FRONT_DAMAGE_SCALE,
    susp_rear_damage_scale = SWOFTY_CAR_SUSP_REAR_DAMAGE_SCALE,
    susp_wheel_detach_scale = SWOFTY_CAR_SUSP_WHEEL_DETACH_SCALE,
    susp_parts_detach_scale = SWOFTY_CAR_SUSP_PARTS_DETACH_SCALE,
    susp_linear_fracture_limit = SWOFTY_CAR_SUSP_LINEAR_FRACTURE_LIMIT,
    susp_angular_fracture_limit = SWOFTY_CAR_SUSP_ANGULAR_FRACTURE_LIMIT,
    engine_limiter = SWOFTY_CAR_ENGINE_LIMITER,                // Engine/LimiterRotationRate (rad/s); the core re-runs the derived limiter
    final_drive = SWOFTY_CAR_FINAL_DRIVE,                      // GearBox/FinalDriveRatio; the core rebuilds the ratio table
    gear_ratio_1 = SWOFTY_CAR_GEAR_RATIO_1,                    // GearBox ratio of 1st .. 9th; the same rebuild
    gear_ratio_2 = SWOFTY_CAR_GEAR_RATIO_1 + 1,
    gear_ratio_3 = SWOFTY_CAR_GEAR_RATIO_1 + 2,
    gear_ratio_4 = SWOFTY_CAR_GEAR_RATIO_1 + 3,
    gear_ratio_5 = SWOFTY_CAR_GEAR_RATIO_1 + 4,
    gear_ratio_6 = SWOFTY_CAR_GEAR_RATIO_1 + 5,
    gear_ratio_7 = SWOFTY_CAR_GEAR_RATIO_1 + 6,
    gear_ratio_8 = SWOFTY_CAR_GEAR_RATIO_1 + 7,
    gear_ratio_9 = SWOFTY_CAR_GEAR_RATIO_1 + 8,
};

}

}

namespace swofty::detail {

inline std::atomic<const SwoftyCarOps *> &car_ops_override()
{
    static std::atomic<const SwoftyCarOps *> t{nullptr};
    return t;
}

inline void set_car_ops_table(const SwoftyCarOps *t) { car_ops_override() = t; }

inline const SwoftyCarOps *car_ops_table()
{
    if (auto t = car_ops_override().load())
        return t->version >= 1 ? t : nullptr;
#if defined(_WIN32)
    static std::atomic<const SwoftyCarOps *> found{nullptr};
    if (auto t = found.load())
        return t;
    HMODULE core = GetModuleHandleW(L"swofty_core.dll");
    auto get = core ? reinterpret_cast<SwoftyCoreCarOpsFn>(GetProcAddress(core, "SwoftyCoreCarOps")) : nullptr;
    const SwoftyCarOps *t = get ? get() : nullptr;
    if (t && t->version < 1)
        t = nullptr;
    found = t;
    return t;
#else
    return nullptr;
#endif
}

inline const SwoftyCarOps *live_car_ops()
{
    auto t = car_ops_table();
    return t && t->available && t->available() ? t : nullptr;
}

}

namespace swofty::cars::live {

class Car {
public:
    Car() = default;
    explicit Car(uint32_t index, void *handle = nullptr) : index_(index), handle_(handle) {}
    uint32_t index() const { return index_; }
    void *handle() const { return handle_; }
    Ref<Vehicle> vehicle() const { return {}; }
    Ref<teams::Identity> team() const { return {}; }
    Ref<drivers::Identity> driver() const { return {}; }
    // Whether the AI drives the car right now (the player's car too, while the AI has it); true when the
    // core can't tell.
    bool is_ai() const
    {
        auto t = swofty::detail::live_car_ops();
        void *h = nullptr;
        uint32_t ai = 1;
        if (!t || !t->car || t->car(index_, &h, &ai) != SWOFTY_ASSET_OK || (handle_ && h != handle_))
            return true;
        return ai == 1;
    }
    std::optional<sessions::ControlState> control() const { return swofty::detail::read_control(index_); }

    std::optional<float> get(Field f) const
    {
        float v = 0;
        return read(f, v) == Result::ok ? std::optional<float>(v) : std::nullopt;
    }
    Result set(Field f, float value) const
    {
        auto t = swofty::detail::live_car_ops();
        if (!t || !t->set)
            return Result::unavailable;
        return swofty::detail::result(t->set(index_, handle_, static_cast<uint32_t>(f), value));
    }
    Result scale(Field f, float factor) const
    {
        float v = 0;
        Result r = read(f, v);
        return r == Result::ok ? set(f, v * factor) : r;
    }

private:
    Result read(Field f, float &v) const
    {
        auto t = swofty::detail::live_car_ops();
        if (!t || !t->get)
            return Result::unavailable;
        return swofty::detail::result(t->get(index_, handle_, static_cast<uint32_t>(f), &v));
    }
    uint32_t index_ = 0;
    void *handle_ = nullptr;
};

inline bool available() { return swofty::detail::live_car_ops() != nullptr; }

inline std::vector<Car> cars()
{
    std::vector<Car> out;
    auto t = swofty::detail::live_car_ops();
    uint32_t n = 0;
    if (!t || !t->count || !t->car || t->count(&n) != SWOFTY_ASSET_OK)
        return out;
    for (uint32_t i = 0; i < n; i++) {
        void *h = nullptr;
        if (t->car(i, &h, nullptr) == SWOFTY_ASSET_OK)
            out.emplace_back(i, h);
    }
    return out;
}

}

namespace swofty::cars::events {

struct UpgradesFit {
    static constexpr const char *point = "car.upgrades.fit";
    static constexpr uint32_t version = 1;
    uint32_t size, payload_version;
    int32_t car;
    const char *vehicle_id;
    Department department;
    float driver_stat;
    float rating;

    Ref<Vehicle> vehicle() const { return swofty::detail::payload_ref<Vehicle>(vehicle_id); }
};

struct VtfRead {
    static constexpr const char *point = "car.vtf.read";
    static constexpr uint32_t version = 1;
    uint32_t size, payload_version;
    live::Car car;
    const char *section_str;
    const char *field_str;
    const char *upgrade_str;
    float value;

    std::string_view section() const { return swofty::detail::payload_str(section_str); }
    std::string_view field() const { return swofty::detail::payload_str(field_str); }
    std::string_view upgrade() const { return swofty::detail::payload_str(upgrade_str); }
};

struct Built {
    static constexpr const char *point = "car.built";
    static constexpr uint32_t version = 1;
    uint32_t size, payload_version;
    live::Car car;
};

struct Reset {
    static constexpr const char *point = "car.reset";
    static constexpr uint32_t version = 1;
    uint32_t size, payload_version;
    live::Car car;
    bool upgrade;
};

struct ControllersRebuilt {
    static constexpr const char *point = "car.controllers.rebuilt";
    static constexpr uint32_t version = 1;
    enum Reason : uint32_t { rebuild = 0, resettle = 1 };
    uint32_t size, payload_version;
    live::Car car;
    uint32_t reason;
};

struct PartsRegister {
    static constexpr const char *point = "car.parts.register";
    static constexpr uint32_t version = 1;
    uint32_t size, payload_version;
    int32_t car;
    const char *vehicle_id;
    void *front_wing;
    int32_t (*add_fn)(void *user, const char *node, float detach_at, bool debris);
    void *user;

    Ref<Vehicle> vehicle() const { return swofty::detail::payload_ref<Vehicle>(vehicle_id); }
    Result add(const Part &part) const
    {
        return add_fn ? swofty::detail::result(add_fn(user, part.node.c_str(), part.detach_at, part.debris))
                      : Result::unavailable;
    }
};

}
