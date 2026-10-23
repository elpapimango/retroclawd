; ste_tone.s: STE DMA sound (S18). `--program ste_tone.bin --org 0x8000` on atari-ste, without TOS.
; A 32-frame stereo buffer at $20000: the left channel a square wave (16 frames +64, 16 frames -64), the right channel
; silent. DMA plays it in a loop at 50066 Hz, so the left channel is a 50066 / 32 = 1564.6 Hz square wave.

    .org $8000
    move.w  #$2700,sr
    lea     $20000,a0
    moveq   #15,d0
high:
    move.b  #$40,(a0)+          ; left
    clr.b   (a0)+               ; right
    dbra    d0,high
    moveq   #15,d0
low:
    move.b  #$c0,(a0)+          ; left, -64
    clr.b   (a0)+
    dbra    d0,low
    move.b  #$02,$ff8903        ; frame base $020000
    clr.b   $ff8905
    clr.b   $ff8907
    move.b  #$02,$ff890f        ; frame end $020040
    clr.b   $ff8911
    move.b  #$40,$ff8913
    move.b  #$03,$ff8921        ; stereo, 50066 Hz
    move.b  #$03,$ff8901        ; play, repeat
forever:
    bra.s   forever
