-- M7: Zones and Plots Migration
-- World expansion: 1000x1000 -> 2000x2000, centered at (1000, 1000)

-- Zone definitions (static reference data)
CREATE TABLE zones (
    id VARCHAR(50) PRIMARY KEY,
    name VARCHAR(100) NOT NULL,
    ring_order INTEGER NOT NULL,  -- 0=Capitol, 6=Wilderness
    inner_radius REAL NOT NULL,
    outer_radius REAL NOT NULL,
    has_plots BOOLEAN NOT NULL DEFAULT true,
    plot_size_category VARCHAR(50),
    deed_cost INTEGER,
    description TEXT
);

-- Plot size categories
CREATE TABLE plot_size_categories (
    id VARCHAR(50) PRIMARY KEY,
    name VARCHAR(100) NOT NULL,
    width REAL NOT NULL,
    height REAL NOT NULL,
    station_capacity INTEGER NOT NULL
);

-- Plots (pre-generated grid)
CREATE TABLE plots (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    zone_id VARCHAR(50) NOT NULL REFERENCES zones(id),
    grid_x INTEGER NOT NULL,
    grid_y INTEGER NOT NULL,
    world_x REAL NOT NULL,
    world_y REAL NOT NULL,
    bounds_min_x REAL NOT NULL,
    bounds_min_y REAL NOT NULL,
    bounds_max_x REAL NOT NULL,
    bounds_max_y REAL NOT NULL,
    size_category VARCHAR(50) NOT NULL REFERENCES plot_size_categories(id),
    plot_type VARCHAR(50) NOT NULL DEFAULT 'residential',
    owner_id UUID REFERENCES players(id) ON DELETE SET NULL,
    claimed_at TIMESTAMPTZ,
    assessed_value INTEGER NOT NULL DEFAULT 0,
    last_tax_paid TIMESTAMPTZ,
    tax_owed INTEGER NOT NULL DEFAULT 0,
    station_count INTEGER NOT NULL DEFAULT 0,
    UNIQUE(zone_id, grid_x, grid_y)
);

-- Plot permissions
CREATE TABLE plot_permissions (
    plot_id UUID PRIMARY KEY REFERENCES plots(id) ON DELETE CASCADE,
    entry_level VARCHAR(50) NOT NULL DEFAULT 'public',
    visibility_level VARCHAR(50) NOT NULL DEFAULT 'public',
    station_use_level VARCHAR(50) NOT NULL DEFAULT 'owner',
    modify_level VARCHAR(50) NOT NULL DEFAULT 'owner'
);

-- Property tax log
CREATE TABLE property_tax_transactions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    plot_id UUID NOT NULL REFERENCES plots(id),
    player_id UUID NOT NULL REFERENCES players(id),
    amount INTEGER NOT NULL,
    tax_week DATE NOT NULL,
    paid_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Add current zone tracking to players
ALTER TABLE players ADD COLUMN current_zone VARCHAR(50);

-- Add plot reference to stations
ALTER TABLE stations ADD COLUMN plot_id UUID REFERENCES plots(id);

-- Indexes for performance
CREATE INDEX idx_plots_zone ON plots(zone_id);
CREATE INDEX idx_plots_owner ON plots(owner_id);
CREATE INDEX idx_plots_bounds ON plots(bounds_min_x, bounds_min_y, bounds_max_x, bounds_max_y);
CREATE INDEX idx_property_tax_plot ON property_tax_transactions(plot_id);
CREATE INDEX idx_property_tax_player ON property_tax_transactions(player_id);

-- Seed zone data (world center at 1000, 1000)
INSERT INTO zones (id, name, ring_order, inner_radius, outer_radius, has_plots, plot_size_category, deed_cost, description) VALUES
    ('capitol', 'The Capitol', 0, 0, 50, false, NULL, NULL, 'The heart of the city. No private ownership allowed.'),
    ('trade_district', 'Trade District', 1, 50, 150, false, NULL, NULL, 'Commercial hub for trading and exchanges.'),
    ('guild_district', 'Guild District', 2, 150, 250, true, 'guild_hall', 5000, 'Territory for established guilds and craftsmen.'),
    ('urban', 'Urban Zone', 3, 250, 400, true, 'medium', 1000, 'Dense residential and commercial area.'),
    ('suburban', 'Suburban Zone', 4, 400, 600, true, 'medium', 500, 'Mixed residential neighborhoods.'),
    ('rural', 'Rural Zone', 5, 600, 850, true, 'large', 250, 'Open lands for farming and resource gathering.'),
    ('wilderness', 'Wilderness', 6, 850, 1000, false, NULL, NULL, 'Untamed lands beyond civilization.');

-- Seed plot size categories
INSERT INTO plot_size_categories (id, name, width, height, station_capacity) VALUES
    ('small', 'Small Plot', 20, 20, 4),
    ('medium', 'Medium Plot', 40, 40, 9),
    ('large', 'Large Plot', 60, 60, 16),
    ('guild_hall', 'Guild Hall', 80, 80, 25);

-- Offset existing positions by +500, +500 to center in new world
UPDATE players SET position_x = position_x + 500, position_y = position_y + 500;
UPDATE players SET destination_x = destination_x + 500 WHERE destination_x IS NOT NULL;
UPDATE players SET destination_y = destination_y + 500 WHERE destination_y IS NOT NULL;
UPDATE resource_nodes SET position_x = position_x + 500, position_y = position_y + 500;

-- Delete existing stations (clean slate for plot-based placement)
-- This is acceptable in alpha - stations will need to be placed on owned plots
DELETE FROM stations;

-- Function to generate plots in a zone
-- This will be called separately via a script or can be run manually
-- For now, we'll generate some initial plots

-- Generate plots for urban zone (ring 250-400)
-- Using a polar grid for better coverage of circular zones
DO $$
DECLARE
    zone_rec RECORD;
    size_rec RECORD;
    angle REAL;
    radius REAL;
    world_x REAL;
    world_y REAL;
    grid_x INTEGER;
    grid_y INTEGER;
    plot_width REAL;
    plot_height REAL;
    center_x CONSTANT REAL := 1000.0;
    center_y CONSTANT REAL := 1000.0;
BEGIN
    -- Generate plots for each zone that has plots
    FOR zone_rec IN SELECT * FROM zones WHERE has_plots = true LOOP
        -- Get the plot size for this zone
        SELECT * INTO size_rec FROM plot_size_categories WHERE id = zone_rec.plot_size_category;
        plot_width := size_rec.width;
        plot_height := size_rec.height;

        -- Calculate number of plots that fit in each ring
        -- Use middle radius for spacing calculation
        radius := (zone_rec.inner_radius + zone_rec.outer_radius) / 2;

        -- Generate plots in concentric rings within the zone
        FOR r IN 1..FLOOR((zone_rec.outer_radius - zone_rec.inner_radius) / plot_height)::INTEGER LOOP
            radius := zone_rec.inner_radius + (r - 0.5) * plot_height;

            -- Calculate number of plots that fit around this ring
            -- Circumference / plot_width
            FOR a IN 0..FLOOR(2 * 3.14159 * radius / plot_width)::INTEGER - 1 LOOP
                angle := (a * plot_width) / radius;

                world_x := center_x + radius * COS(angle);
                world_y := center_y + radius * SIN(angle);

                -- Use ring and angle index as grid coordinates
                grid_x := a;
                grid_y := r;

                INSERT INTO plots (
                    zone_id, grid_x, grid_y, world_x, world_y,
                    bounds_min_x, bounds_min_y, bounds_max_x, bounds_max_y,
                    size_category, plot_type
                ) VALUES (
                    zone_rec.id, grid_x, grid_y, world_x, world_y,
                    world_x - plot_width/2, world_y - plot_height/2,
                    world_x + plot_width/2, world_y + plot_height/2,
                    zone_rec.plot_size_category, 'residential'
                );
            END LOOP;
        END LOOP;
    END LOOP;
END $$;

-- Create default permissions for all plots
INSERT INTO plot_permissions (plot_id)
SELECT id FROM plots;
