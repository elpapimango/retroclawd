; pce-vdcwait: what an access to the video chips costs the CPU. The timer runs with reload 0 (an interrupt every 1024 fast
; cycles, counted in $40-$41 by the handler) while each of six loops makes 2048 x 8 accesses of one kind; the count after each
; loop is stored at $50 + 2 x n (low byte first):
;   0 stz $2100 (work RAM)        1 stz $0002 (VDC data)      2 stz $0404 (VCE data)
;   3 lda $0000 (VDC status)      4 stz $0800 (sound, the CPU's own port)       5 lda $2100 (work RAM)
;   6 st1 #0 (2048 x 8)
; and then the block transfers, timed by the timer's counter ($70-$75: before and after tii, tia to the VDC, tin to the VDC).
; The difference between a loop's count and loop 0's (or 5's) is the extra time of its 16384 accesses, in units of 1024 cycles.
; -D DISPLAY=1 turns the display on (background and sprites, the usual 256 x 240 timing) before the loops.
    .include "common.inc"
    .ifndef DISPLAY
DISPLAY = 0
    .endif
    .org $E000

.macro run n
    stz $40
    stz $41
    ldx #8
    ldy #0
.endm

.macro done n
    lda $40
    sta $50+2*n
    lda $41
    sta $51+2*n
.endm

start:
    preamble
    .if DISPLAY
    vdc $0A,$02,$02
    vdc $0B,$1F,$04
    vdc $0C,$02,$0F
    vdc $0D,$EF,$00
    vdc $0E,$03,$00
    vdc $05,$C0,$00
    .endif
    st0 #0
    st1 #0
    st2 #0
    st0 #2
    lda #$01
    sta $1402
    stz $0C00
    lda #$01
    sta $0C01
    sta $1403
    cli

    run 0
l0:
    stz $2100
    stz $2100
    stz $2100
    stz $2100
    stz $2100
    stz $2100
    stz $2100
    stz $2100
    dey
    bne l0
    dex
    bne l0
    done 0

    run 1
l1:
    stz $0002
    stz $0002
    stz $0002
    stz $0002
    stz $0002
    stz $0002
    stz $0002
    stz $0002
    dey
    bne l1
    dex
    bne l1
    done 1

    run 2
l2:
    stz $0404
    stz $0404
    stz $0404
    stz $0404
    stz $0404
    stz $0404
    stz $0404
    stz $0404
    dey
    bne l2
    dex
    bne l2
    done 2

    run 3
l3:
    lda $0000
    lda $0000
    lda $0000
    lda $0000
    lda $0000
    lda $0000
    lda $0000
    lda $0000
    dey
    bne l3
    dex
    bne l3
    done 3

    run 4
l4:
    stz $0800
    stz $0800
    stz $0800
    stz $0800
    stz $0800
    stz $0800
    stz $0800
    stz $0800
    dey
    bne l4
    dex
    bne l4
    done 4

    run 5
l5:
    lda $2100
    lda $2100
    lda $2100
    lda $2100
    lda $2100
    lda $2100
    lda $2100
    lda $2100
    dey
    bne l5
    dex
    bne l5
    done 5

    run 6
l6:
    st1 #0
    st1 #0
    st1 #0
    st1 #0
    st1 #0
    st1 #0
    st1 #0
    st1 #0
    dey
    bne l6
    dex
    bne l6
    done 6

    ; block transfers hold interrupts off: time four transfers of 3840 bytes each by the timer's counter instead (reload
    ; $7F, no interrupt), read before into $70 + 2 x k and after into $71 + 2 x k
    sei
    lda #$05
    sta $1402
    lda #$7F
    sta $0C00

    stz $0C01
    lda #$01
    sta $0C01
    lda $0C00
    sta $70
    tii $E000,$3000,$0F00
    tii $E000,$3000,$0F00
    tii $E000,$3000,$0F00
    tii $E000,$3000,$0F00
    lda $0C00
    sta $71

    stz $0C01
    lda #$01
    sta $0C01
    lda $0C00
    sta $72
    tia $E000,$0002,$0F00
    tia $E000,$0002,$0F00
    tia $E000,$0002,$0F00
    tia $E000,$0002,$0F00
    lda $0C00
    sta $73

    stz $0C01
    lda #$01
    sta $0C01
    lda $0C00
    sta $74
    tin $E000,$0002,$0F00
    tin $E000,$0002,$0F00
    tin $E000,$0002,$0F00
    tin $E000,$0002,$0F00
    lda $0C00
    sta $75

    sei
spin:
    bra spin

isr:
    inc $40
    bne isr1
    inc $41
isr1:
    stz $1403
    rti

    .org $FFF6
    .word $EAEA,$EAEA,isr,$EAEA,start
