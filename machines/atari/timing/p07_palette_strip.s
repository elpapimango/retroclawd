; P07: a palette write part way along a line.
;
; The screen is filled with colour 1 (white). On 64 lines (100 to 163 after the VBL) the HBL handler runs a NOP slide
; of 4 * k clocks (k = 0 to 63, one more on each line), writes palette entry 1 red, waits 100 clocks and writes it white
; again: a red strip on each line, its start `entry + 4 k` clocks into the line. The picture shows where the colour change
; reaches the beam. Nothing is measured in memory; compare the screenshots (timing/analyse_strips.py).
    .org $90000
    move.w  #$2700,sr
    clr.b   $fffa07             ; IERA, IERB, IMRA, IMRB: no MFP interrupt reaches the CPU
    clr.b   $fffa09
    clr.b   $fffa13
    clr.b   $fffa15
    moveq   #0,d0
    move.b  $ff8201,d0          ; the video base: high byte, middle byte, 0
    lsl.l   #8,d0
    move.b  $ff8203,d0
    lsl.l   #8,d0
    movea.l d0,a0
    move.w  #3999,d1
fill:
    move.l  #$ffff0000,(a0)+    ; plane 0 all ones: colour 1
    clr.l   (a0)+
    dbf     d1,fill
    clr.w   $ff8240             ; colour 0 black
    move.w  #$777,$ff8242       ; colour 1 white
    lea     hbl(pc),a0
    move.l  a0,$68
    lea     vbl(pc),a0
    move.l  a0,$70
    moveq   #0,d6               ; frames
    moveq   #0,d7               ; lines since the VBL
    move.w  #$2100,sr
idle:
    stop    #$2100
    bra.s   idle

vbl:
    move.w  #$2100,(sp)
    addq.w  #1,d6
    moveq   #0,d7
    rte

hbl:
    addq.w  #1,d7
    move.w  d7,d0
    sub.w   #100,d0
    cmp.w   #64,d0
    bcc.w   out
    add.w   d0,d0
    lea     slide_end(pc),a1
    suba.w  d0,a1
    jmp     (a1)
slide:                          ; 64 NOPs
    dc.w    $4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71
    dc.w    $4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71
    dc.w    $4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71
    dc.w    $4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71
slide_end:
    move.w  #$700,$ff8242       ; red
    dc.w    $4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71
    dc.w    $4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71
    move.w  #$777,$ff8242       ; white again
out:
    rte
