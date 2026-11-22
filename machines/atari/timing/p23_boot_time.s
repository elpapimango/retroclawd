; p23_boot_time.s: how long EmuTOS takes to get to the floppy's boot sector and the body after it.
; The body (run by the boot stub after Floprd read it) stores TOS's 200 Hz tick counter ($4BA) and the frame counter ($466)
; at its start, again after 100 000 counted loop runs, and the 200 Hz counter at 8 later points (each 10 000 runs apart).
; Result at $C000: header 'P23', then the 200 Hz counter, the frame counter, the 200 Hz counter and the frame counter after
; the loop, then eight more 200 Hz counters, as longs.
    .org $90000
    lea     $c000,a2
    move.l  #$50323300,(a2)+
    move.l  $4ba,(a2)+
    move.l  $466,(a2)+
    move.l  #100000,d5
l1:
    subq.l  #1,d5
    bne.s   l1
    move.l  $4ba,(a2)+
    move.l  $466,(a2)+
    moveq   #7,d7
more:
    move.l  #10000,d5
l2:
    subq.l  #1,d5
    bne.s   l2
    move.l  $4ba,(a2)+
    dbra    d7,more
done:
    bra.s   done
