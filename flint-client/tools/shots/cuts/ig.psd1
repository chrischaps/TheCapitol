# The Instagram reel (#343): 1080x1920, about 40 s, from the rim inward
# over one day. In and Out are seconds into the shot's clip; Fade is the
# crossfade into the next segment (the cut's Fade by default). A
# segment's titles are timed from its own start, the cut's from its
# start (or, negative, from its end), and so are the score's. Sizes and heights are at 1080x1920.
@{
    Shape = "portrait"
    Fade = 0.5
    Shade = @{ Top = 0.32; Bottom = 0.0; Alpha = 0.55 }
    Card = @{ At = -9.0; Dim = 0.5 }
    Cover = 3.0

    Segments = @(
        @{ Shot = "wilds-range"; In = 0.3; Out = 5.5
           Titles = @(@{ Text = "Start with nothing."; At = 0.7; Until = 4.9; Size = 70; Y = 300 }) }
        @{ Shot = "grass-dawn"; In = 0.3; Out = 5.0
           Titles = @(@{ Text = "Gather what grows."; At = 0.6; Until = 4.4; Size = 70; Y = 300 }) }
        @{ Shot = "houses-grow"; In = 0.3; Out = 5.8
           Titles = @(@{ Text = "Claim a plot."; At = 0.6; Until = 5.0; Size = 70; Y = 280 }
                      @{ Text = "Watch a home rise."; At = 1.4; Until = 5.0; Size = 70; Y = 380 }) }
        @{ Shot = "bazaar-street"; In = 0.3; Out = 5.6
           Titles = @(@{ Text = "Trade with everyone."; At = 0.6; Until = 4.8; Size = 70; Y = 300 }) }
        @{ Shot = "tower-climb"; In = 0.8; Out = 6.3
           Titles = @(@{ Text = "Work your way inward."; At = 0.6; Until = 4.9; Size = 66; Y = 300 }) }
        @{ Shot = "clock-dawn"; In = 0; Out = 1.75
           Titles = @(@{ Text = "One city. One clock."; At = 0.5; Until = 5.5; Size = 70; Y = 300 }) }
        @{ Shot = "clock-noon"; In = 0; Out = 1.75 }
        @{ Shot = "clock-dusk"; In = 0; Out = 1.75 }
        @{ Shot = "clock-night"; In = 0; Out = 2.25 }
        @{ Shot = "night-arc"; In = 0; Out = 12
           Titles = @(@{ Text = "End with everything."; At = 0.6; Until = 3.0; Size = 70; Y = 300 }) }
    )

    # The end card, over the night
    Titles = @(
        @{ Text = "THE CAPITOL"; At = -8.3; Size = 112; Y = 760 }
        @{ Text = "An economy you can stand in."; Font = "Sans"; Color = "0xF2EEE6"; At = -7.6; Size = 48; Y = 910 }
        @{ Text = "In development  ·  chaps.dev"; Font = "Sans"; Color = "0xBFB8AA"; At = -6.9; Size = 38; Y = 990 }
    )

    # The game's own sounds (--write-sounds), each timed from its segment
    Score = @(
        @{ Sound = "amb_breeze"; Loop = $true; Seg = "wilds-range"; For = 10; Volume = 0.9; FadeIn = 1.5; FadeOut = 2.5 }
        @{ Sound = "amb_stream"; Loop = $true; Seg = "wilds-range"; For = 6; Volume = 0.35; FadeIn = 1.0; FadeOut = 2.0 }
        @{ Sound = "bird_1"; Seg = "wilds-range"; At = 1.6; Volume = 0.6 }
        @{ Sound = "bird_3"; Seg = "wilds-range"; At = 3.4; Volume = 0.5 }
        @{ Sound = "rustle_2"; Seg = "grass-dawn"; At = 1.4; Volume = 0.8 }
        @{ Sound = "pluck"; Seg = "grass-dawn"; At = 2.2; Volume = 0.7 }
        @{ Sound = "bird_4"; Seg = "grass-dawn"; At = 3.3; Volume = 0.5 }
        @{ Sound = "amb_city"; Loop = $true; Seg = "houses-grow"; For = 6; Volume = 0.8; FadeIn = 1.5; FadeOut = 1.5 }
        @{ Sound = "chime"; Seg = "houses-grow"; At = 2.0; Volume = 0.8 }
        @{ Sound = "amb_market"; Loop = $true; Seg = "bazaar-street"; For = 5.5; Volume = 0.9; FadeIn = 1.0; FadeOut = 1.5 }
        @{ Sound = "hawker_2"; Seg = "bazaar-street"; At = 1.2; Volume = 0.7 }
        @{ Sound = "coins"; Seg = "bazaar-street"; At = 3.0; Volume = 0.7 }
        @{ Sound = "amb_wind"; Loop = $true; Seg = "tower-climb"; For = 7; Volume = 0.7; FadeIn = 1.5; FadeOut = 2.0 }
        @{ Sound = "amb_city"; Loop = $true; Seg = "clock-dawn"; For = 7; Volume = 0.6; FadeIn = 1.5; FadeOut = 2.0 }
        @{ Sound = "amb_water"; Loop = $true; Seg = "night-arc"; Volume = 0.6; FadeIn = 2.5 }
        @{ Sound = "bell"; At = -9.2; Volume = 2.4 }
    )
}
