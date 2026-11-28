; pce-hpos: where the picture lands across the screen for given display registers. The background is a 32 x 32 map with a
; white cell in column 0 and column (HDW) of every row, so the first and last displayed columns show; the backdrop is blue.
; -D HSR=$xxyy (HDS in the high byte, HSW in the low), -D HDR=$xxyy (HDE high, HDW low), -D DOT=0..2 (the dot clock).
    .include "common.inc"
    .ifndef HSR
HSR = $0202
    .endif
    .ifndef HDR
HDR = $041F
    .endif
    .ifndef DOT
DOT = 0
    .endif
    .org $E000

.macro vreg reg,lo,hi
    st0 #reg
    lda #lo
    sta $0002
    lda #hi
    sta $0003
.endm

start:
    preamble
    ; dot clock
    lda #DOT
    sta $0400
    ; palette: backdrop blue, colour 1 white, border black
    put $00,$0402
    put $00,$0403
    put $07,$0404
    put $00,$0405
    put $FF,$0404
    put $01,$0405
    put $00,$0402
    put $01,$0403
    put $00,$0404
    put $00,$0405
    ; tile $100 at $1000: colour 1; tile $101 at $1010: empty
    vreg $00,$00,$10
    st0 #$02
    ldx #8
solid:
    put $FF,$0002
    put $00,$0003
    dex
    bne solid
    ldx #24
empty:
    put $00,$0002
    put $00,$0003
    dex
    bne empty
    ; the map: each row, cell 0 and cell HDW solid, the rest empty
    vreg $00,$00,$00
    st0 #$02
    ldy #32
row:
    ldx #0
cell:
    lda #$01               ; empty tile $101
    cpx #0
    beq solidcell
    cpx #(HDR & $7F)
    bne put_it
solidcell:
    lda #$00               ; tile $100
put_it:
    sta $0002
    lda #$01
    sta $0003
    inx
    cpx #32
    bne cell
    dey
    bne row
    ; display
    vreg $0A,(HSR & $FF),(HSR >> 8)
    vreg $0B,(HDR & $FF),(HDR >> 8)
    vreg $0C,$02,$0F
    vreg $0D,$EF,$00
    vreg $0E,$03,$00
    vreg $09,$00,$00
    vreg $07,$00,$00
    vreg $08,$00,$00
    vreg $05,$80,$00
spin:
    bra spin
    .org $FFF6
    .word $EAEA,$EAEA,$EAEA,$EAEA,start
