; P06: the MFP timer A data register against the time since the control register started the timer.
;
; For each of four prescalers (/4, /10, /50, /200) and for k = 0 to 63: the timer is stopped, its data register loaded
; with 255, the control register written to start it, then k NOPs (4 clocks each) and the data register read. The read
; value says how many timer clocks went by in 4k + the instruction clocks: with the prescaler's phase and the data
; register's read slot it is a fingerprint of the MFP model.
;
; Result at $C000: 'P06' 0, then 64 bytes for each prescaler in that order.
    .org $90000
    move.w  #$2700,sr
    clr.b   $fffa07             ; IERA, IERB, IMRA, IMRB: no MFP interrupt reaches the CPU
    clr.b   $fffa09
    clr.b   $fffa13
    clr.b   $fffa15
    lea     $c000,a0
    move.l  #$50303600,(a0)+    ; "P06" 0
    lea     modes(pc),a2
    moveq   #3,d4               ; prescalers left
next_mode:
    move.b  (a2)+,d5            ; the control register value
    moveq   #0,d3               ; k
loop:
    clr.b   $fffa19             ; stop timer A
    move.b  #255,$fffa1f        ; data register
    move.b  d5,$fffa19          ; start it
    lea     slide_end(pc),a1
    move.w  d3,d0
    add.w   d0,d0
    suba.w  d0,a1
    jmp     (a1)
slide:                          ; 64 NOPs
    dc.w    $4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71
    dc.w    $4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71
    dc.w    $4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71
    dc.w    $4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71,$4E71
slide_end:
    move.b  $fffa1f,(a0)+
    addq.w  #1,d3
    cmp.w   #64,d3
    bne.w   loop
    dbf     d4,next_mode
finished:
    bra.s   finished
modes:
    dc.b    1,2,5,7,0,0
