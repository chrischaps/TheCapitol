# The Instagram reel (#343): 1080x1920, about 35 s. Only what the first
# reel (Oct 18) didn't show: the wilds, the farms, the bazaar and the
# citizens living in it, from the edge of the world at dawn to the same
# ridge at night with the whole city lit. In and Out are seconds into the
# shot's clip; Fade is the crossfade into the next segment (the cut's
# Fade by default). A segment's titles are timed from its own start, the
# cut's from its start (or, negative, from its end), and so are the
# score's. Sizes and heights are at 1080x1920.
@{
    Shape = "portrait"
    Fade = 0.5
    Shade = @{ Top = 0.32; Bottom = 0.0; Alpha = 0.55 }
    Card = @{ At = -8.0; Dim = 0.45 }
    Cover = 6.5

    Segments = @(
        @{ Shot = "wilds-range"; In = 0.3; Out = 5.5
           Titles = @(@{ Text = "Start at the edge."; At = 0.7; Until = 4.9; Size = 70; Y = 300 }) }
        @{ Shot = "wilds-ridge"; In = 0.3; Out = 5.0
           Titles = @(@{ Text = "Cross the wilds."; At = 0.6; Until = 4.4; Size = 70; Y = 300 }) }
        @{ Shot = "wilds-lakes"; In = 0.3; Out = 4.5 }
        @{ Shot = "farms-lane"; In = 0.3; Out = 5.0
           Titles = @(@{ Text = "Put down roots."; At = 0.6; Until = 4.4; Size = 70; Y = 300 }) }
        @{ Shot = "bazaar-street"; In = 0.3; Out = 5.6
           Titles = @(@{ Text = "Trade with everyone."; At = 0.6; Until = 4.8; Size = 70; Y = 300 }) }
        @{ Shot = "citizens"; In = 0.3; Out = 5.3
           Titles = @(@{ Text = "Live among citizens."; At = 0.6; Until = 4.6; Size = 70; Y = 300 }) }
        @{ Shot = "wilds-ridge-night"; In = 0; Out = 12
           Titles = @(@{ Text = "End with everything."; At = 0.6; Until = 3.4; Size = 70; Y = 300 }) }
    )

    # The end card, over the city lit at the centre of the wilds
    Titles = @(
        @{ Text = "THE CAPITOL"; At = -7.3; Size = 112; Y = 760 }
        @{ Text = "An economy you can stand in."; Font = "Sans"; Color = "0xF2EEE6"; At = -6.6; Size = 48; Y = 910 }
        @{ Text = "In development  ·  chaps.dev"; Font = "Sans"; Color = "0xBFB8AA"; At = -5.9; Size = 38; Y = 990 }
    )

    # The game's own sounds (--write-sounds), each timed from its segment
    Score = @(
        @{ Sound = "amb_wind"; Loop = $true; Seg = "wilds-range"; For = 10; Volume = 0.7; FadeIn = 1.5; FadeOut = 2.5 }
        @{ Sound = "bird_2"; Seg = "wilds-range"; At = 2.0; Volume = 0.4 }
        @{ Sound = "amb_stream"; Loop = $true; Seg = "wilds-ridge"; For = 8; Volume = 0.5; FadeIn = 1.5; FadeOut = 2.0 }
        @{ Sound = "amb_breeze"; Loop = $true; Seg = "farms-lane"; For = 5; Volume = 0.9; FadeIn = 1.0; FadeOut = 1.5 }
        @{ Sound = "bird_1"; Seg = "farms-lane"; At = 1.0; Volume = 0.6 }
        @{ Sound = "bird_3"; Seg = "farms-lane"; At = 2.8; Volume = 0.5 }
        @{ Sound = "amb_market"; Loop = $true; Seg = "bazaar-street"; For = 10; Volume = 0.9; FadeIn = 1.0; FadeOut = 2.0 }
        @{ Sound = "hawker_2"; Seg = "bazaar-street"; At = 1.2; Volume = 0.7 }
        @{ Sound = "coins"; Seg = "bazaar-street"; At = 3.0; Volume = 0.7 }
        @{ Sound = "hawker_3"; Seg = "citizens"; At = 1.6; Volume = 0.6 }
        @{ Sound = "amb_wind"; Loop = $true; Seg = "wilds-ridge-night"; Volume = 0.45; FadeIn = 2.5 }
        @{ Sound = "amb_city"; Loop = $true; Seg = "wilds-ridge-night"; Volume = 0.35; FadeIn = 3.0 }
        @{ Sound = "bell"; At = -8.2; Volume = 2.4 }
    )
}
