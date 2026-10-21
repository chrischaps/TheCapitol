# The YouTube teaser (#343): 3840x2160, about 90 s. One day from dawn
# at the rim to night at the tower, ring by ring, each ring named once
# in a quiet lower third. The game's own sounds are the score. Format as
# ig.psd1; a score layer's Seg times it from that shot's segment, and
# For is how long it plays.
@{
    Shape = "landscape"
    Fade = 1.0
    Shade = @{ Top = 0.0; Bottom = 0.3; Alpha = 0.5 }
    Card = @{ At = -9.0; Dim = 0.5 }
    Cover = 6.0

    Segments = @(
        @{ Shot = "wilds-range"; In = 0.3; Out = 8.8
           Titles = @(@{ Text = "THE WILDS"; At = 1.5; Until = 7.0; Size = 120; Y = 1800 }) }
        @{ Shot = "wilds-ridge"; In = 0.3; Out = 7.8 }
        @{ Shot = "wilds-rim"; In = 0.3; Out = 6.8 }
        @{ Shot = "wilds-ford"; In = 0.3; Out = 6.8 }
        @{ Shot = "farms-high"; In = 0.3; Out = 8.3
           Titles = @(@{ Text = "THE FARMS"; At = 1.2; Until = 6.8; Size = 120; Y = 1800 }) }
        @{ Shot = "farms-lane"; In = 0.3; Out = 7.3 }
        @{ Shot = "houses-grow"; In = 0.3; Out = 6.8
           Titles = @(@{ Text = "THE SUBURBS"; At = 1.2; Until = 5.6; Size = 120; Y = 1800 }) }
        @{ Shot = "suburb-gold"; In = 0.3; Out = 8.3 }
        @{ Shot = "automation"; In = 0.5; Out = 7.5 }
        @{ Shot = "bazaar-street"; In = 0.3; Out = 7.3
           Titles = @(@{ Text = "THE BAZAAR"; At = 1.2; Until = 6.0; Size = 120; Y = 1800 }) }
        @{ Shot = "hall-night"; In = 0.3; Out = 6.3 }
        @{ Shot = "tower-climb"; In = 0.8; Out = 7.8
           Titles = @(@{ Text = "THE CAPITOL"; At = 1.2; Until = 6.0; Size = 120; Y = 1800 }) }
        @{ Shot = "night-arc"; In = 0; Out = 12 }
    )

    Titles = @(
        @{ Text = "THE CAPITOL"; At = -8.3; Size = 210; Y = 900 }
        @{ Text = "An economy you can stand in."; Font = "Sans"; Color = "0xF2EEE6"; At = -7.6; Size = 86; Y = 1170 }
        @{ Text = "In development  ·  chaps.dev"; Font = "Sans"; Color = "0xBFB8AA"; At = -6.9; Size = 66; Y = 1300 }
    )

    Score = @(
        # The wilds: wind on the ridges, then running water at the rim and the ford
        @{ Sound = "amb_wind"; Loop = $true; Seg = "wilds-range"; For = 16; Volume = 0.7; FadeIn = 2.5; FadeOut = 3.0 }
        @{ Sound = "amb_stream"; Loop = $true; Seg = "wilds-rim"; For = 13; Volume = 0.55; FadeIn = 2.0; FadeOut = 2.5 }
        @{ Sound = "bird_2"; Seg = "wilds-ridge"; At = 2.0; Volume = 0.4 }
        @{ Sound = "bird_4"; Seg = "wilds-ford"; At = 1.5; Volume = 0.5 }
        # The farms: the breeze in the grass, birds
        @{ Sound = "amb_breeze"; Loop = $true; Seg = "farms-high"; For = 16; Volume = 0.9; FadeIn = 2.0; FadeOut = 3.0 }
        @{ Sound = "bird_1"; Seg = "farms-high"; At = 2.0; Volume = 0.6 }
        @{ Sound = "bird_3"; Seg = "farms-high"; At = 5.5; Volume = 0.5 }
        @{ Sound = "rustle_1"; Seg = "farms-lane"; At = 1.0; Volume = 0.7 }
        @{ Sound = "bird_4"; Seg = "farms-lane"; At = 3.5; Volume = 0.5 }
        # The suburbs and the work: the city's hum, a chime as the houses rise
        @{ Sound = "amb_city"; Loop = $true; Seg = "houses-grow"; For = 40; Volume = 0.8; FadeIn = 3.0; FadeOut = 3.0 }
        @{ Sound = "chime"; Seg = "houses-grow"; At = 2.0; Volume = 0.8 }
        @{ Sound = "pluck"; Seg = "automation"; At = 2.5; Volume = 0.6 }
        # The tower: wind high up, then the water of the moat and the bell
        @{ Sound = "amb_wind"; Loop = $true; Seg = "tower-climb"; For = 10; Volume = 0.6; FadeIn = 2.0; FadeOut = 3.0 }
        @{ Sound = "amb_water"; Loop = $true; Seg = "night-arc"; Volume = 0.6; FadeIn = 3.0 }
        @{ Sound = "bell"; At = -9.2; Volume = 2.4 }
    )
}
