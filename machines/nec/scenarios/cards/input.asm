; pce-input: shows the first pad's eight inputs as eight bars on the background layer: draws a palette, one solid tile and a map,
; then each frame sets the eight map cells from the pad (button I, II, select, run, up, right, down, left order: bits of the two reads).
    .include "common.inc"
    .org $E000

; a bar: select map word col in row 4 of the 32-wide map, then write tile 1 (pressed) or 0 from A
.macro bar_start col
    st0 #$00               ; MAWR
    lda #col
    sta $0002
    lda #$01
    sta $0003
    st0 #$02               ; VWR
.endm
.macro bar_end
    and #$01               ; 1 = released
    eor #$01               ; 1 = pressed
    sta $0002
    lda #$00
    sta $0003
.endm
.macro pad_read
    lda #$01
    sta $1000              ; the port needs time to settle: CLR, then SEL, as games do
    lda #$03
    sta $1000
    lda #$01
    sta $1000
    pha
    pla
    nop
.endm

start:
    preamble
    ; palette: entry $100 (the border) = black
    put $00,$0402
    put $01,$0403
    put $00,$0404
    put $00,$0405
    ; palette: entry 0 = black, entry 1 = white
    put $00,$0402
    put $00,$0403
    put $00,$0404
    put $00,$0405
    put $FF,$0404
    put $01,$0405
    ; VRAM is not cleared at power on: blank tile 0 and the whole first 32x32 map (words 0-$3FF)
    st0 #$00
    lda #$00
    sta $0002
    sta $0003
    st0 #$02
    ldx #$00
clear:
    lda #$00
    sta $0002
    sta $0003
    lda #$00
    sta $0002
    sta $0003
    lda #$00
    sta $0002
    sta $0003
    lda #$00
    sta $0002
    sta $0003
    inx
    bne clear
    ldx #$FF
    ; tile 1 at VRAM words 16-31: plane 0 all ones
    st0 #$00
    lda #$10
    sta $0002
    lda #$00
    sta $0003
    st0 #$02
    put $FF,$0002
    put $00,$0003
    put $FF,$0002
    put $00,$0003
    put $FF,$0002
    put $00,$0003
    put $FF,$0002
    put $00,$0003
    put $FF,$0002
    put $00,$0003
    put $FF,$0002
    put $00,$0003
    put $FF,$0002
    put $00,$0003
    put $FF,$0002
    put $00,$0003
    put $00,$0002
    sta $0003
    put $00,$0002
    sta $0003
    put $00,$0002
    sta $0003
    put $00,$0002
    sta $0003
    put $00,$0002
    sta $0003
    put $00,$0002
    sta $0003
    put $00,$0002
    sta $0003
    put $00,$0002
    sta $0003
    ; control: background on, vblank interrupt off
    st0 #$05
    lda #$80
    sta $0002
    lda #$00
    sta $0003
    ; display registers: 256 wide, 224 high
    vdc $0B,$1F,$00
    vdc $0C,$02,$0F
    vdc $0D,$DF,$00
    vdc $0E,$04,$00
    vdc $09,$00,$00
frame:
    pad_read
    lda $1000
    sta $01                ; directions in bits 0-3 (up right down left)
    lda #$00
    sta $1000
    pha
    pla
    nop
    lda $1000
    sta $00                ; buttons in bits 0-3 (I II select run)
    bar_start $40
    lda $00
    bar_end
    bar_start $42
    lda $00
    lsr a
    bar_end
    bar_start $44
    lda $00
    lsr a
    lsr a
    bar_end
    bar_start $46
    lda $00
    lsr a
    lsr a
    lsr a
    bar_end
    bar_start $48
    lda $01
    bar_end
    bar_start $4A
    lda $01
    lsr a
    bar_end
    bar_start $4C
    lda $01
    lsr a
    lsr a
    bar_end
    bar_start $4E
    lda $01
    lsr a
    lsr a
    lsr a
    bar_end
    jmp frame
    .org $FFF6
    .word $EAEA,$EAEA,$EAEA,$EAEA,start
