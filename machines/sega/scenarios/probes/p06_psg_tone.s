; P06: a PSG square wave on channel 0 at full volume, period 200 (clock / (32 * 200) = 559 Hz), the other channels silent.
; Nothing is stored: the audio is compared (level and pitch) between the emulators.
    .org $200
    move.b  #$80+$00,$C00011   ; tone 0, low bits of the period ($C8 & $F = 8)
    move.b  #$88,$C00011
    move.b  #$0C,$C00011       ; high bits ($C8 >> 4 = $C)
    move.b  #$90,$C00011       ; tone 0 attenuation 0 (full)
    move.b  #$BF,$C00011       ; tone 1 off
    move.b  #$DF,$C00011       ; tone 2 off
    move.b  #$FF,$C00011       ; noise off
forever:
    bra.s   forever
