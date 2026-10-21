# The Instagram reel (#343): 1080x1920, about 35 s, from the rim inward
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

    # The game's own sounds (--write-sounds)
    Score = @(
        @{ Sound = "amb_breeze"; Loop = $true; At = 0; Until = 15; Volume = 0.9; FadeIn = 1.5; FadeOut = 2.5 }
        @{ Sound = "amb_stream"; Loop = $true; At = 0; Until = 6; Volume = 0.35; FadeIn = 1.0; FadeOut = 2.0 }
        @{ Sound = "bird_1"; At = 1.6; Volume = 0.6 }
        @{ Sound = "bird_3"; At = 3.4; Volume = 0.5 }
        @{ Sound = "rustle_2"; At = 6.2; Volume = 0.8 }
        @{ Sound = "pluck"; At = 7.0; Volume = 0.7 }
        @{ Sound = "bird_4"; At = 8.1; Volume = 0.5 }
        @{ Sound = "amb_city"; Loop = $true; At = 9.0; Until = 24; Volume = 0.8; FadeIn = 2.0; FadeOut = 2.0 }
        @{ Sound = "chime"; At = 11.5; Volume = 0.8 }
        @{ Sound = "amb_wind"; Loop = $true; At = 13.5; Until = 21; Volume = 0.7; FadeIn = 2.0; FadeOut = 2.0 }
        @{ Sound = "amb_water"; Loop = $true; At = 19.5; Volume = 0.6; FadeIn = 2.5; FadeOut = 2.0 }
        @{ Sound = "bell"; At = -9.2; Volume = 2.4 }
    )
}
