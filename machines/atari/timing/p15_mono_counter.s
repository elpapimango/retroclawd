; p15_mono_counter.s: the video address counter at the entry of the HBL handler of every line of one frame on the
; monochrome monitor; it shows on which line the picture's data starts and stops being fetched and how much a line fetches.
; Frame 2 is recorded (the HBL is masked in frames 1 and 3). Result at $C000: header 'P15', then 3 bytes (high, mid, low)
; for each HBL of the frame, in order.
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
    lea     $c000,a2
    move.l  #$50313500,(a2)+
    moveq   #0,d6               ; frames
    move.w  #$2300,sr
idle:
    bra.s   idle                ; a loop, not STOP: STOP would set the mask again after every interrupt

vbl:
    addq.w  #1,d6
    move.w  #$2300,(sp)
    cmp.w   #2,d6
    bne.s   notrec
    move.w  #$2100,(sp)         ; frame 2: the HBL is taken
notrec:
    cmp.w   #3,d6
    bne.s   done
    ori.w   #$0700,(sp)         ; frame 3: everything masked, the frame is recorded
done:
    rte

hbl:
    move.b  $ff8205,(a2)+
    move.b  $ff8207,(a2)+
    move.b  $ff8209,(a2)+
    rte
