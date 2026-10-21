# The YouTube tour (#343): 3840x2160, about two and a quarter minutes.
# The teaser's journey from the rim to the tower, with a caption on each
# beat saying what the game does there. A segment's Chapter starts a
# YouTube chapter (renders\v2\tour-chapters.txt): the first at 0:00,
# each at least 10 s. The samples are captioned as samples where it
# matters. Format as ig.psd1 and yt.psd1.
@{
    Shape = "landscape"
    Fade = 0.8
    Shade = @{ Top = 0.0; Bottom = 0.34; Alpha = 0.6 }
    Card = @{ At = -9.0; Dim = 0.5 }
    Cover = 4.0

    Segments = @(
        @{ Shot = "wilds-range"; In = 0.3; Out = 9.3; Chapter = "The edge of the world"
           Titles = @(@{ Text = "An economy you can stand in"; At = 1.0; Until = 8.2; Size = 104; Y = 1660 }
                      @{ Text = "No combat. Everything is gathered, made and traded by the people who live here."; Font = "Sans"; Color = "0xF2EEE6"; At = 1.6; Until = 8.2; Size = 60; Y = 1810 }) }
        @{ Shot = "wilds-ridge"; In = 0.3; Out = 8.3
           Titles = @(@{ Text = "Start at the edge"; At = 0.8; Until = 7.2; Size = 104; Y = 1660 }
                      @{ Text = "Everyone begins out here with nothing, and works inward to the tower."; Font = "Sans"; Color = "0xF2EEE6"; At = 1.4; Until = 7.2; Size = 60; Y = 1810 }) }
        @{ Shot = "wilds-lakes"; In = 0.3; Out = 7.3
           Titles = @(@{ Text = "The wilds"; At = 0.8; Until = 6.2; Size = 104; Y = 1660 }
                      @{ Text = "Iron high on the rock, old growth in the valleys, clay on the banks. Quality rises as you climb."; Font = "Sans"; Color = "0xF2EEE6"; At = 1.4; Until = 6.2; Size = 60; Y = 1810 }) }
        @{ Shot = "wilds-ford"; In = 0.3; Out = 6.3
           Titles = @(@{ Text = "Rivers stop you. Fords and bridges let you through."; Font = "Sans"; Color = "0xF2EEE6"; At = 0.8; Until = 5.2; Size = 60; Y = 1810 }) }

        @{ Shot = "farms-high"; In = 0.3; Out = 8.3; Chapter = "Farms and fiber"
           Titles = @(@{ Text = "The farms"; At = 0.8; Until = 7.2; Size = 104; Y = 1660 }
                      @{ Text = "Farms on straight roads and winding lanes, laid out round the water."; Font = "Sans"; Color = "0xF2EEE6"; At = 1.4; Until = 7.2; Size = 60; Y = 1810 }) }
        @{ Shot = "grass-dawn"; In = 0.3; Out = 6.3
           Titles = @(@{ Text = "Gather"; At = 0.8; Until = 5.2; Size = 104; Y = 1660 }
                      @{ Text = "Fiber is the grass. One Fiber backs one Strand, so every coin in the world was earned."; Font = "Sans"; Color = "0xF2EEE6"; At = 1.4; Until = 5.2; Size = 60; Y = 1810 }) }
        @{ Shot = "gather-juice"; In = 0.5; Out = 9.5
           Titles = @(@{ Text = "A scripted sample of what a gather feels like"; Font = "Sans"; Color = "0xBFB8AA"; At = 0.6; Until = 8.0; Size = 52; Y = 1890 }) }

        @{ Shot = "craft-panel"; In = 0.3; Out = 6.0; Chapter = "Craft and quality"
           Titles = @(@{ Text = "Craft"; At = 0.6; Until = 5.0; Size = 104; Y = 1660 }
                      @{ Text = "Quality carries down the chain: the inputs, the station and the tool. Grades run A to F."; Font = "Sans"; Color = "0xF2EEE6"; At = 1.2; Until = 5.0; Size = 60; Y = 1810 }) }
        @{ Shot = "automation"; In = 0.5; Out = 8.5
           Titles = @(@{ Text = "Stations work while you're away"; At = 0.8; Until = 7.4; Size = 104; Y = 1660 }
                      @{ Text = "Forges, kilns and anvils run a recipe from their own box, and every unit keeps its maker's mark."; Font = "Sans"; Color = "0xF2EEE6"; At = 1.4; Until = 7.4; Size = 60; Y = 1810 }) }

        @{ Shot = "houses-grow"; In = 0.3; Out = 6.8; Chapter = "Land and houses"
           Titles = @(@{ Text = "Claim a plot"; At = 0.8; Until = 5.8; Size = 104; Y = 1660 }
                      @{ Text = "Every claimed lot grows a house, the same for everyone who looks."; Font = "Sans"; Color = "0xF2EEE6"; At = 1.4; Until = 5.8; Size = 60; Y = 1810 }) }
        @{ Shot = "suburb-gold"; In = 0.3; Out = 8.3
           Titles = @(@{ Text = "Land pays tax by the hour. Fall four weeks behind and the plot goes back on the market."; Font = "Sans"; Color = "0xF2EEE6"; At = 0.8; Until = 7.2; Size = 60; Y = 1810 }) }

        @{ Shot = "hall-night"; In = 0.3; Out = 7.3; Chapter = "The Exchange"
           Titles = @(@{ Text = "The Exchange"; At = 0.8; Until = 6.2; Size = 104; Y = 1660 }
                      @{ Text = "One order book for the whole world. Buy by grade; every order held in escrow."; Font = "Sans"; Color = "0xF2EEE6"; At = 1.4; Until = 6.2; Size = 60; Y = 1810 }) }
        @{ Shot = "market-panel"; In = 0.3; Out = 6.0 }

        @{ Shot = "bazaar-street"; In = 0.3; Out = 7.3; Chapter = "The bazaar and the Gazette"
           Titles = @(@{ Text = "The bazaar"; At = 0.8; Until = 6.2; Size = 104; Y = 1660 }
                      @{ Text = "Forty-one stalls. Rent one by the day, name your shop, set your own prices."; Font = "Sans"; Color = "0xF2EEE6"; At = 1.4; Until = 6.2; Size = 60; Y = 1810 }) }
        # The page fills the frame's height, so its words sit in the left margin
        @{ Shot = "gazette-page"; In = 0.3; Out = 7.0
           Titles = @(@{ Text = "The Capitol"; At = 0.8; Until = 5.9; Size = 96; Y = 760; X = 150 }
                      @{ Text = "Gazette"; At = 0.8; Until = 5.9; Size = 96; Y = 880; X = 150 }
                      @{ Text = "Every game day:"; Font = "Sans"; Color = "0xF2EEE6"; At = 1.4; Until = 5.9; Size = 56; Y = 1060; X = 150 }
                      @{ Text = "the prices that moved,"; Font = "Sans"; Color = "0xF2EEE6"; At = 1.4; Until = 5.9; Size = 56; Y = 1135; X = 150 }
                      @{ Text = "the finest sale"; Font = "Sans"; Color = "0xF2EEE6"; At = 1.4; Until = 5.9; Size = 56; Y = 1210; X = 150 }
                      @{ Text = "and who made it,"; Font = "Sans"; Color = "0xF2EEE6"; At = 1.4; Until = 5.9; Size = 56; Y = 1285; X = 150 }
                      @{ Text = "the deeds, the work."; Font = "Sans"; Color = "0xF2EEE6"; At = 1.4; Until = 5.9; Size = 56; Y = 1360; X = 150 }) }

        @{ Shot = "citizens"; In = 0.3; Out = 6.3; Chapter = "Citizens and the clock"
           Titles = @(@{ Text = "Citizens"; At = 0.6; Until = 5.4; Size = 104; Y = 1660 }
                      @{ Text = "AI players who earn everything a person does, marked by a pale lantern."; Font = "Sans"; Color = "0xF2EEE6"; At = 1.0; Until = 5.4; Size = 60; Y = 1810 }) }
        @{ Shot = "clock-dawn"; In = 0; Out = 1.75; Fade = 0.5
           Titles = @(@{ Text = "One city. One clock."; At = 0.5; Until = 5.4; Size = 104; Y = 1660 }
                      @{ Text = "Day and night follow the server, so everyone shares the hour."; Font = "Sans"; Color = "0xF2EEE6"; At = 0.9; Until = 5.4; Size = 60; Y = 1810 }) }
        @{ Shot = "clock-noon"; In = 0; Out = 1.75; Fade = 0.5 }
        @{ Shot = "clock-dusk"; In = 0; Out = 1.75; Fade = 0.5 }
        @{ Shot = "clock-night"; In = 0; Out = 2.25 }

        @{ Shot = "tower-climb"; In = 0.8; Out = 7.8; Chapter = "The Capitol"
           Titles = @(@{ Text = "Work your way inward"; At = 0.8; Until = 6.6; Size = 104; Y = 1660 }) }
        @{ Shot = "night-arc"; In = 0; Out = 12 }
    )

    Titles = @(
        @{ Text = "THE CAPITOL"; At = -8.3; Size = 210; Y = 860 }
        @{ Text = "An economy you can stand in."; Font = "Sans"; Color = "0xF2EEE6"; At = -7.6; Size = 86; Y = 1130 }
        @{ Text = "In development  ·  chaps.dev"; Font = "Sans"; Color = "0xBFB8AA"; At = -6.9; Size = 66; Y = 1260 }
        @{ Text = "Built in Rust on the Flint engine, with Claude"; Font = "Sans"; Color = "0x8F887C"; At = -6.2; Size = 54; Y = 1360 }
    )

    Score = @(
        @{ Sound = "amb_wind"; Loop = $true; Seg = "wilds-range"; For = 20; Volume = 0.6; FadeIn = 2.5; FadeOut = 3.0 }
        @{ Sound = "amb_stream"; Loop = $true; Seg = "wilds-lakes"; For = 15; Volume = 0.5; FadeIn = 2.0; FadeOut = 2.5 }
        @{ Sound = "bird_2"; Seg = "wilds-ridge"; At = 2.5; Volume = 0.4 }
        @{ Sound = "amb_breeze"; Loop = $true; Seg = "farms-high"; For = 24; Volume = 0.85; FadeIn = 2.0; FadeOut = 3.0 }
        @{ Sound = "bird_1"; Seg = "farms-high"; At = 2.0; Volume = 0.6 }
        @{ Sound = "bird_3"; Seg = "grass-dawn"; At = 1.5; Volume = 0.5 }
        @{ Sound = "rustle_1"; Seg = "gather-juice"; At = 1.0; Volume = 0.7 }
        @{ Sound = "pluck"; Seg = "gather-juice"; At = 1.6; Volume = 0.6 }
        @{ Sound = "rustle_3"; Seg = "gather-juice"; At = 4.0; Volume = 0.7 }
        @{ Sound = "pluck"; Seg = "gather-juice"; At = 4.6; Volume = 0.6 }
        @{ Sound = "amb_city"; Loop = $true; Seg = "craft-panel"; For = 70; Volume = 0.7; FadeIn = 3.0; FadeOut = 3.0 }
        @{ Sound = "ui_click"; Seg = "craft-panel"; At = 3.0; Volume = 0.8 }
        @{ Sound = "chime"; Seg = "houses-grow"; At = 2.0; Volume = 0.8 }
        @{ Sound = "coins"; Seg = "market-panel"; At = 2.0; Volume = 0.7 }
        @{ Sound = "amb_market"; Loop = $true; Seg = "bazaar-street"; For = 7; Volume = 0.9; FadeIn = 1.5; FadeOut = 2.0 }
        @{ Sound = "hawker_1"; Seg = "bazaar-street"; At = 1.5; Volume = 0.7 }
        @{ Sound = "amb_market"; Loop = $true; Seg = "citizens"; For = 6; Volume = 0.7; FadeIn = 1.0; FadeOut = 2.0 }
        @{ Sound = "hawker_3"; Seg = "citizens"; At = 2.0; Volume = 0.6 }
        @{ Sound = "amb_wind"; Loop = $true; Seg = "tower-climb"; For = 10; Volume = 0.6; FadeIn = 2.0; FadeOut = 3.0 }
        @{ Sound = "amb_water"; Loop = $true; Seg = "night-arc"; Volume = 0.6; FadeIn = 3.0 }
        @{ Sound = "bell"; At = -9.2; Volume = 2.4 }
    )
}
