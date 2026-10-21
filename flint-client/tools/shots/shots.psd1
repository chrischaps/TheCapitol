# The shot library for tools\teaser.ps1 (#343). Each shot is a camera
# path in this folder, a time of day (0 midnight, 0.25 dawn, 0.5 noon,
# 0.8 evening) and how many seconds to film. Optional:
#   Portrait  another path for the 1080x1920 frame (keep the tower within
#             about 10 degrees of centre there)
#   Args      sample flags (--claim-sample 0.6, --grow-at 1, ...)
#   Input     an input script in this folder
#   Size      the landscape frame, in place of 3840x2160: panels are
#             drawn at the window's scale, so UI shots film at 1920x1080
#   Shapes    which frames -Only renders (portrait, landscape)
# Filming starts half a second in; a path's times are game time.
@{
    Shots = @(
        # The wilds, from the rim inward
        @{ Name = "wilds-range";  Path = "wilds-range.txt";  Time = 0.26; Length = 9; Shapes = "portrait", "landscape" }
        @{ Name = "wilds-ridge";  Path = "wilds-ridge.txt";  Time = 0.28; Length = 9; Shapes = "portrait", "landscape" }
        @{ Name = "wilds-valley"; Path = "wilds-valley.txt"; Time = 0.30; Length = 8; Shapes = "landscape" }
        @{ Name = "wilds-rim";    Path = "wilds-rim.txt";    Time = 0.34; Length = 8; Shapes = "landscape" }
        @{ Name = "wilds-ford";   Path = "wilds-ford.txt";   Time = 0.40; Length = 8; Shapes = "portrait", "landscape" }
        @{ Name = "wilds-lakes";  Path = "wilds-lakes.txt";  Time = 0.66; Length = 8; Shapes = "portrait", "landscape" }
        @{ Name = "wilds-ridge-night"; Path = "wilds-ridge-night.txt"; Time = 0.92; Length = 12; Args = "--claim-sample", "0.6"; Shapes = "portrait", "landscape" }

        # The farms (#290) and the grass; claimed farms grow a farmhouse and a barn
        @{ Name = "farms-high";   Path = "farms-high.txt";   Time = 0.56; Length = 9; Args = "--claim-sample", "0.6"; Shapes = "portrait", "landscape" }
        @{ Name = "farms-lane";   Path = "farms-lane.txt";   Time = 0.60; Length = 8; Args = "--claim-sample", "0.6"; Shapes = "portrait", "landscape" }
        @{ Name = "grass-dawn";   Path = "grass-dawn.txt";   Time = 0.27; Length = 7; Shapes = "portrait", "landscape" }

        # The suburbs: building, and the hours over them
        @{ Name = "houses-grow";  Path = "houses-grow.txt";  Time = 0.70; Length = 7; Args = "--claim-sample", "0.3", "--grow-at", "1.0"; Shapes = "portrait", "landscape" }
        @{ Name = "suburb-gold";  Path = "suburb-gold.txt";  Time = 0.72; Length = 9; Args = "--claim-sample", "0.6"; Shapes = "landscape" }
        @{ Name = "clock-dawn";   Path = "one-clock.txt";    Time = 0.27; Length = 2.5; Args = "--claim-sample", "0.6"; Shapes = "portrait", "landscape" }
        @{ Name = "clock-noon";   Path = "one-clock.txt";    Time = 0.50; Length = 2.5; Args = "--claim-sample", "0.6"; Shapes = "portrait", "landscape" }
        @{ Name = "clock-dusk";   Path = "one-clock.txt";    Time = 0.76; Length = 2.5; Args = "--claim-sample", "0.6"; Shapes = "portrait", "landscape" }
        @{ Name = "clock-night";  Path = "one-clock.txt";    Time = 0.92; Length = 2.5; Args = "--claim-sample", "0.6"; Shapes = "portrait", "landscape" }

        # Work and trade
        @{ Name = "automation";   Path = "automation.txt";   Time = 0.82; Length = 10; Args = @("--automation-sample"); Shapes = "landscape" }
        @{ Name = "citizens";     Path = "citizens.txt";     Time = 0.86; Length = 7; Args = "--citizen-sample", "stroll", "--bazaar-sample", "street"; Shapes = "portrait", "landscape" }
        @{ Name = "bazaar-street"; Path = "bazaar-street.txt"; Time = 0.84; Length = 8; Args = "--bazaar-sample", "street", "--citizen-sample", "stroll"; Shapes = "portrait", "landscape" }
        @{ Name = "bazaar-above"; Path = "bazaar-above.txt"; Time = 0.84; Length = 8; Args = "--bazaar-sample", "street"; Shapes = "landscape" }
        @{ Name = "hall-night";   Path = "hall-night.txt";   Time = 0.88; Length = 7; Args = "--claim-sample", "0.6"; Shapes = "landscape" }

        # The panels, for the tour: filmed at 2560x1440 so a panel leaves room for a caption
        @{ Name = "gather-juice"; Path = "panel-rural.txt";  Time = 0.30; Length = 10; Args = @("--juice-sample"); Size = 2560, 1440; Shapes = "landscape" }
        @{ Name = "craft-panel";  Path = "panel-rural.txt";  Time = 0.75; Length = 6; Args = "--station-panel-sample", "craft"; Size = 2560, 1440; Shapes = "landscape" }
        @{ Name = "market-panel"; Path = "panel-hall.txt";   Time = 0.88; Length = 6; Args = @("--market-sample"); Size = 2560, 1440; Shapes = "landscape" }
        @{ Name = "gazette-page"; Path = "panel-gazette.txt"; Time = 0.82; Length = 7; Args = @("--gazette-sample"); Size = 2560, 1440; Shapes = "landscape" }

        # The tower and the city by night
        @{ Name = "tower-climb";  Path = "tower-climb.txt";  Time = 0.78; Length = 8; Shapes = "portrait", "landscape" }
        @{ Name = "night-arc";    Path = "night-arc.txt";    Time = 0.90; Length = 12; Args = "--claim-sample", "0.6"; Shapes = "portrait", "landscape" }
        @{ Name = "night-hero";   Path = "night-hero.txt";   Time = 0.90; Length = 6; Args = "--claim-sample", "0.6"; Shapes = "landscape" }
        @{ Name = "city-crane";   Path = "city-crane.txt";   Time = 0.74; Length = 11; Args = "--claim-sample", "0.6"; Shapes = "landscape" }
    )
}
