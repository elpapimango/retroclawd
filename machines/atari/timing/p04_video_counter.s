; P04: the video address counter ($FF8205/07/09) along a line.
;
; The CPU waits in STOP, so an HBL interrupt reaches the handler after a fixed latency whatever the beam is doing: the
; handler enters at a fixed point of the line. Only a few target lines are sampled; every other HBL returns at once, so
; interrupts never pile up. On a target line the handler runs a NOP slide whose length grows by 4 clocks on each frame
; (128 frames, 0 to 508 clocks), then reads the three counter bytes. A sample therefore says what the counter held at
; (line start + latency + 4 * frame) clocks on that line.
;
; Result at $C000: 'P04' 0, then per sample five bytes: the frame number (1 to 128), the line, the counter high
; ($FF8205), mid ($FF8207) and low ($FF8209) bytes, in the order the HBLs came.
    .org $90000
    move.w  #$2700,sr
    clr.b   $fffa07             ; IERA, IERB, IMRA, IMRB: no MFP interrupt reaches the CPU
    clr.b   $fffa09
    clr.b   $fffa13
    clr.b   $fffa15
    lea     hbl(pc),a0
    move.l  a0,$68
    lea     vbl(pc),a0
    move.l  a0,$70
    lea     $d000,a3            ; the table of target lines: one byte a line
    move.w  #159,d0
clear:
    clr.w   (a3)+
    dbf     d0,clear
    lea     $d000,a3
    st      10(a3)              ; a line in the top border
    st      40(a3)              ; where the picture usually starts
    st      100(a3)
    st      250(a3)
    st      290(a3)             ; below the picture
    lea     $c000,a2
    move.l  #$50303400,(a2)+    ; "P04" 0
    moveq   #0,d6               ; frames done
    moveq   #0,d7               ; lines since the VBL
    lea     slide_end(pc),a1
    move.w  #$2100,sr           ; HBL (2) and VBL (4) may interrupt
idle:
    stop    #$2100
    bra.s   idle

vbl:
    addq.w  #1,d6
    moveq   #0,d7
    move.w  d6,d0
    subq.w  #1,d0
    and.w   #127,d0             ; this frame's step: 4 clocks a step, counted back from the end of the slide
    add.w   d0,d0
    lea     slide_end(pc),a1
    suba.w  d0,a1
    cmp.w   #129,d6
    bne.s   vbl_done
    ori.w   #$0700,(sp)         ; the frames are done: mask every interrupt
vbl_done:
    rte

hbl:
    addq.w  #1,d7
    tst.b   0(a3,d7.w)
    bne.s   target
    rte
target:
    jmp     (a1)
slide:                          ; 128 NOPs
    dc.w    $4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71
    dc.w    $4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71
    dc.w    $4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71
    dc.w    $4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71
    dc.w    $4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71
    dc.w    $4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71
    dc.w    $4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71
    dc.w    $4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71
slide_end:
    move.b  d6,(a2)+
    move.b  d7,(a2)+
    move.b  $ff8205,(a2)+
    move.b  $ff8207,(a2)+
    move.b  $ff8209,(a2)+
    rte
